//! Picture tools: look at, resize/convert, sort and clean photos in the workspace.
//!
//!   image_info             safe      size, dimensions, date taken, camera, has location?
//!   resize_image           mutating  resized / converted COPIES (never overwrites)
//!   organize_photos        mutating  moves photos into YYYY/YYYY-MM folders by date taken
//!   remove_photo_location  mutating  strips location + camera data, losslessly
//!   image_thumbnail        (UI only) small JPEG preview for the Live view
//!
//! Safety / privacy properties:
//!  - every path goes through Workspace::resolve; folder listings skip links
//!  - image_info reports only WHETHER a photo has GPS, never the coordinates:
//!    the exact spot (often the user's home) is not sent to the model provider
//!  - decoding is capped (pixels and memory) so a "decompression bomb" image
//!    can't exhaust RAM; batch tools cap the number of files per call
//!  - remove_photo_location edits JPEG/PNG containers WITHOUT re-compressing
//!    (no quality loss), keeps the picture's rotation, writes to a temp file
//!    and renames, and re-reads the result to confirm the location is gone
//!  - mutating tools are snapshotted by the caller, so "Undo this run" works

use crate::workspace::Workspace;
use serde::Serialize;
use std::fs;
use std::io::{BufReader, Cursor, Write};
use std::path::{Path, PathBuf};

const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp", "tif", "tiff", "heic", "heif"];
const DECODABLE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp", "tif", "tiff"];
const MAX_FILES: usize = 200;
const MAX_PIXELS: u64 = 80_000_000; // ~80 MP
const MAX_ALLOC: u64 = 768 * 1024 * 1024;

// ---------------------------------------------------------------
// helpers
// ---------------------------------------------------------------

fn ext_of(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase()
}

fn is_image(p: &Path) -> bool {
    IMAGE_EXTS.contains(&ext_of(p).as_str())
}

/// Workspace-relative display path with forward slashes.
fn rel_of(ws: &Workspace, p: &Path) -> String {
    p.strip_prefix(ws.root()).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

/// A file, or the images directly inside a folder (not recursive, no links).
fn collect(ws: &Workspace, relative_path: &str) -> Result<(Vec<PathBuf>, bool), String> {
    let path = ws.resolve(relative_path)?;
    if !path.exists() {
        return Err(format!("'{}' does not exist.", relative_path));
    }
    if path.is_file() {
        if !is_image(&path) {
            return Err(format!("'{}' is not a picture (supported: {}).", relative_path, IMAGE_EXTS.join(", ")));
        }
        return Ok((vec![path], false));
    }
    let mut out = Vec::new();
    for e in fs::read_dir(&path).map_err(|e| format!("Could not read '{}': {}", relative_path, e))?.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_file() && !ft.is_symlink() && is_image(&e.path()) {
            out.push(e.path());
        }
    }
    out.sort();
    if out.is_empty() {
        let hint = folders_with_pictures(ws);
        return Err(format!(
            "'{}' contains no pictures.{}",
            relative_path,
            if hint.is_empty() { " No folder in the workspace has pictures.".to_string() } else { format!(" Pictures are in: {}.", hint.join(", ")) }
        ));
    }
    if out.len() > MAX_FILES {
        return Err(format!("'{}' has {} pictures; the limit is {} per call. Work on a smaller folder.", relative_path, out.len(), MAX_FILES));
    }
    Ok((out, true))
}

/// Folders (up to 3 levels deep) that directly contain pictures, e.g.
/// ["photos (7)", ". (2)"], so the model can fix a wrong folder name itself.
/// Found live: "copies of all photos in a folder called small" made the agent
/// look for photos IN small/ and give up, although photos/ had 7.
fn folders_with_pictures(ws: &Workspace) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![(ws.root().to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        let mut n = 0;
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let name = e.file_name().to_string_lossy().to_string();
            if ft.is_dir() && !ft.is_symlink() && depth < 3 && !name.starts_with('.') && name != "node_modules" && name != "target" {
                stack.push((e.path(), depth + 1));
            } else if ft.is_file() && is_image(&e.path()) {
                n += 1;
            }
        }
        if n > 0 {
            let rel = rel_of(ws, &dir);
            found.push(format!("{} ({})", if rel.is_empty() { ".".to_string() } else { rel }, n));
        }
    }
    found.sort();
    found.truncate(8);
    found
}

/// `name.ext`, then `name (1).ext`, ... — never overwrites.
fn unique(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{}.{}", stem, ext));
    if !first.exists() {
        return first;
    }
    (1..10_000).map(|i| dir.join(format!("{} ({}).{}", stem, i, ext))).find(|p| !p.exists()).unwrap_or(first)
}

/// Lenient: keeps every field it could read even if part of the EXIF block is
/// malformed. Found while testing: a strict parse rejected a whole (valid
/// enough) EXIF block over one bad IFD pointer and reported "no location".
fn lenient(r: Result<exif::Exif, exif::Error>) -> Option<exif::Exif> {
    r.or_else(|e| e.distill_partial_result(|_| {})).ok()
}

fn read_exif_file(p: &Path) -> Option<exif::Exif> {
    let f = fs::File::open(p).ok()?;
    lenient(exif::Reader::new().continue_on_error(true).read_from_container(&mut BufReader::new(f)))
}

fn read_exif_bytes(b: &[u8]) -> Option<exif::Exif> {
    lenient(exif::Reader::new().continue_on_error(true).read_from_container(&mut Cursor::new(b)))
}

/// The file has an EXIF block (whether or not we could read it).
fn has_exif_block(p: &Path) -> bool {
    use std::io::Read;
    let mut head = vec![0u8; 128 * 1024];
    let n = fs::File::open(p).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    head[..n].windows(6).any(|w| w == b"Exif\0\0") || head[..n].windows(4).any(|w| w == b"eXIf")
}

/// "2019:07:14 10:22:33" -> ("2019", "07", "14")
fn exif_date(x: &exif::Exif) -> Option<(String, String, String)> {
    for tag in [exif::Tag::DateTimeOriginal, exif::Tag::DateTimeDigitized, exif::Tag::DateTime] {
        if let Some(f) = x.get_field(tag, exif::In::PRIMARY) {
            if let exif::Value::Ascii(v) = &f.value {
                if let Some(raw) = v.first() {
                    let s = String::from_utf8_lossy(raw);
                    let d: Vec<&str> = s.get(0..10).unwrap_or("").split(|c| c == ':' || c == '-').collect();
                    if d.len() == 3 && d[0].len() == 4 && d[0] != "0000" && d.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())) {
                        return Some((d[0].to_string(), d[1].to_string(), d[2].to_string()));
                    }
                }
            }
        }
    }
    None
}

fn has_gps(x: &exif::Exif) -> bool {
    x.get_field(exif::Tag::GPSLatitude, exif::In::PRIMARY).is_some()
        || x.get_field(exif::Tag::GPSLongitude, exif::In::PRIMARY).is_some()
}

fn orientation(x: &exif::Exif) -> u32 {
    x.get_field(exif::Tag::Orientation, exif::In::PRIMARY).and_then(|f| f.value.get_uint(0)).unwrap_or(1)
}

fn ascii(x: &exif::Exif, tag: exif::Tag) -> Option<String> {
    let f = x.get_field(tag, exif::In::PRIMARY)?;
    let s = f.display_value().to_string().trim_matches('"').trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

/// Decodes with limits (pixels + memory) so a hostile image can't eat the RAM.
fn decode(p: &Path) -> Result<image::DynamicImage, String> {
    let (w, h) = image::image_dimensions(p).map_err(|e| format!("Could not read '{}': {}", p.display(), e))?;
    if (w as u64) * (h as u64) > MAX_PIXELS {
        return Err(format!("{}x{} is too large to process (limit {} MP).", w, h, MAX_PIXELS / 1_000_000));
    }
    let mut r = image::ImageReader::open(p).map_err(|e| e.to_string())?.with_guessed_format().map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_ALLOC);
    r.limits(limits);
    r.decode().map_err(|e| format!("Could not decode '{}': {}", p.display(), e))
}

/// Applies the EXIF rotation so resized copies come out the right way up.
fn upright(img: image::DynamicImage, orient: u32) -> image::DynamicImage {
    match orient {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

// ---------------------------------------------------------------
// image_info
// ---------------------------------------------------------------

#[derive(Serialize, Debug)]
pub struct ImageInfo {
    pub path: String,
    pub format: String,
    pub size_bytes: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub taken: Option<String>,
    pub camera: Option<String>,
    pub has_location: bool,
    /// Has an EXIF block we could not read: fail safe, treat as "may contain location".
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub location_unknown: bool,
}

#[derive(Serialize, Debug)]
pub struct ImageInfoResult {
    pub images: Vec<ImageInfo>,
    pub count: usize,
    pub with_location: usize,
    pub note: String,
}

fn info_for(ws: &Workspace, p: &Path) -> ImageInfo {
    let x = read_exif_file(p);
    let (width, height) = match image::image_dimensions(p) {
        Ok((w, h)) => (Some(w), Some(h)),
        Err(_) => (None, None),
    };
    let camera = x.as_ref().and_then(|x| {
        let make = ascii(x, exif::Tag::Make).unwrap_or_default();
        let model = ascii(x, exif::Tag::Model).unwrap_or_default();
        let s = if model.to_lowercase().starts_with(&make.to_lowercase()) { model } else { format!("{} {}", make, model) };
        let s = s.trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    });
    ImageInfo {
        path: rel_of(ws, p),
        format: ext_of(p),
        size_bytes: fs::metadata(p).map(|m| m.len()).unwrap_or(0),
        width,
        height,
        taken: x.as_ref().and_then(exif_date).map(|(y, m, d)| format!("{}-{}-{}", y, m, d)),
        camera,
        has_location: x.as_ref().map(has_gps).unwrap_or(false),
        location_unknown: x.is_none() && has_exif_block(p),
    }
}

pub fn image_info(ws: &Workspace, relative_path: &str) -> Result<ImageInfoResult, String> {
    let (files, _) = collect(ws, relative_path)?;
    let images: Vec<ImageInfo> = files.iter().map(|p| info_for(ws, p)).collect();
    let with_location = images.iter().filter(|i| i.has_location).count();
    let unknown = images.iter().filter(|i| i.location_unknown).count();
    let mut note = if with_location > 0 {
        format!("{} picture(s) contain GPS location data (where they were taken). Exact coordinates are not shown. Warn the user before these are shared; remove_photo_location can strip it.", with_location)
    } else {
        "No GPS location data found.".to_string()
    };
    if unknown > 0 {
        note.push_str(&format!(" {} picture(s) have hidden metadata that could not be read, so they MAY contain a location; remove_photo_location strips it anyway.", unknown));
    }
    Ok(ImageInfoResult {
        count: images.len(),
        with_location,
        note,
        images,
    })
}

// ---------------------------------------------------------------
// resize_image
// ---------------------------------------------------------------

#[derive(Serialize, Debug)]
pub struct ResizedFile {
    pub from: String,
    pub to: String,
    pub before: String,
    pub after: String,
    pub size_bytes: u64,
}

#[derive(Serialize, Debug)]
pub struct ResizeResult {
    pub created: Vec<ResizedFile>,
    pub skipped: Vec<String>,
    pub note: String,
}

/// New size: fit inside max_width x max_height (never enlarge), or a percent.
pub fn target_size(w: u32, h: u32, max_w: Option<u32>, max_h: Option<u32>, percent: Option<u32>) -> (u32, u32) {
    if let Some(p) = percent {
        let p = p.clamp(1, 200) as f64 / 100.0;
        return (((w as f64) * p).round().max(1.0) as u32, ((h as f64) * p).round().max(1.0) as u32);
    }
    let sw = max_w.map(|m| m as f64 / w as f64).unwrap_or(1.0);
    let sh = max_h.map(|m| m as f64 / h as f64).unwrap_or(1.0);
    let s = sw.min(sh).min(1.0);
    (((w as f64) * s).round().max(1.0) as u32, ((h as f64) * s).round().max(1.0) as u32)
}

pub fn check_resize_args(max_w: Option<u32>, max_h: Option<u32>, percent: Option<u32>, format: Option<&str>) -> Result<(), String> {
    if let Some(f) = format {
        if !["jpg", "jpeg", "png", "webp"].contains(&f.to_lowercase().as_str()) {
            return Err("format must be jpg, png or webp.".into());
        }
    }
    if max_w.is_none() && max_h.is_none() && percent.is_none() && format.is_none() {
        return Err("Say what to change: max_width / max_height, percent, or format.".into());
    }
    for v in [max_w, max_h].into_iter().flatten() {
        if !(8..=10_000).contains(&v) {
            return Err("max_width / max_height must be between 8 and 10000 pixels.".into());
        }
    }
    if let Some(p) = percent {
        if !(1..=200).contains(&p) {
            return Err("percent must be between 1 and 200.".into());
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn resize_image(
    ws: &Workspace,
    relative_path: &str,
    max_width: Option<u32>,
    max_height: Option<u32>,
    percent: Option<u32>,
    format: Option<String>,
    quality: Option<u32>,
    output_folder: Option<String>,
) -> Result<ResizeResult, String> {
    check_resize_args(max_width, max_height, percent, format.as_deref())?;
    let (files, _) = collect(ws, relative_path)?;
    let out_dir = match output_folder.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(f) => {
            let d = ws.resolve(f)?;
            fs::create_dir_all(&d).map_err(|e| format!("Could not create '{}': {}", f, e))?;
            Some(d)
        }
        None => None,
    };
    let quality = quality.unwrap_or(85).clamp(30, 100) as u8;
    let only_convert = max_width.is_none() && max_height.is_none() && percent.is_none();
    let mut created = Vec::new();
    let mut skipped = Vec::new();
    for p in &files {
        let rel = rel_of(ws, p);
        if !DECODABLE_EXTS.contains(&ext_of(p).as_str()) {
            skipped.push(format!("{} (.{} can't be decoded)", rel, ext_of(p)));
            continue;
        }
        let img = match decode(p) {
            Ok(i) => upright(i, read_exif_file(p).as_ref().map(orientation).unwrap_or(1)),
            Err(e) => {
                skipped.push(format!("{} ({})", rel, e));
                continue;
            }
        };
        let (w, h) = (img.width(), img.height());
        let (nw, nh) = target_size(w, h, max_width, max_height, percent);
        let img = if (nw, nh) == (w, h) { img } else { img.resize_exact(nw, nh, image::imageops::FilterType::CatmullRom) };
        let src_ext = ext_of(p);
        let ext = match format.as_deref().map(|f| f.to_lowercase()) {
            Some(f) if f == "jpeg" => "jpg".to_string(),
            Some(f) => f,
            None if ["jpg", "jpeg"].contains(&src_ext.as_str()) => "jpg".into(),
            None if src_ext == "webp" => "webp".into(),
            None => "png".into(),
        };
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
        let label = if only_convert { "converted".to_string() } else { format!("{}x{}", nw, nh) };
        let dir = out_dir.clone().unwrap_or_else(|| p.parent().unwrap_or(ws.root()).to_path_buf());
        let dest = unique(&dir, &format!("{}-{}", stem, label), &ext);
        let mut buf = Vec::new();
        let enc = match ext.as_str() {
            "jpg" => image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality).encode_image(&img.to_rgb8()),
            "webp" => img.to_rgba8().write_to(&mut Cursor::new(&mut buf), image::ImageFormat::WebP),
            _ => img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png),
        };
        if let Err(e) = enc {
            skipped.push(format!("{} (could not encode: {})", rel, e));
            continue;
        }
        fs::write(&dest, &buf).map_err(|e| format!("Could not write '{}': {}", rel_of(ws, &dest), e))?;
        created.push(ResizedFile { from: rel, to: rel_of(ws, &dest), before: format!("{}x{}", w, h), after: format!("{}x{}", nw, nh), size_bytes: buf.len() as u64 });
    }
    Ok(ResizeResult {
        note: format!("Created {} new file(s); originals were not changed. New copies carry no location or camera data.", created.len()),
        created,
        skipped,
    })
}

// ---------------------------------------------------------------
// organize_photos
// ---------------------------------------------------------------

#[derive(Serialize, Debug)]
pub struct PhotoMove {
    pub from: String,
    pub to: String,
    pub date_from: String, // "photo" (EXIF date taken) | "file date"
}

#[derive(Serialize, Debug)]
pub struct OrganizeResult {
    pub moves: Vec<PhotoMove>,
    pub dry_run: bool,
    pub folders: Vec<String>,
    pub note: String,
}

pub fn organize_photos(ws: &Workspace, relative_path: &str, by: &str, dry_run: bool) -> Result<OrganizeResult, String> {
    let (files, is_folder) = collect(ws, relative_path)?;
    if !is_folder {
        return Err("organize_photos works on a folder of pictures, not a single file.".into());
    }
    let by = by.to_lowercase();
    if !["year", "month", "day"].contains(&by.as_str()) {
        return Err("by must be year, month or day.".into());
    }
    let base = ws.resolve(relative_path)?;
    let mut moves = Vec::new();
    let mut folders = std::collections::BTreeSet::new();
    for p in &files {
        let (date, source) = match read_exif_file(p).as_ref().and_then(exif_date) {
            Some(d) => (d, "photo"),
            None => {
                let t: chrono::DateTime<chrono::Local> = fs::metadata(p).and_then(|m| m.modified()).map(Into::into).unwrap_or_else(|_| chrono::Local::now());
                ((t.format("%Y").to_string(), t.format("%m").to_string(), t.format("%d").to_string()), "file date")
            }
        };
        let (y, m, d) = date;
        let mut dir = base.join(&y);
        if by != "year" {
            dir = dir.join(format!("{}-{}", y, m));
        }
        if by == "day" {
            dir = dir.join(format!("{}-{}-{}", y, m, d));
        }
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("photo");
        let dest = unique(&dir, stem, &ext_of(p));
        folders.insert(rel_of(ws, &dir));
        if !dry_run {
            fs::create_dir_all(&dir).map_err(|e| format!("Could not create '{}': {}", rel_of(ws, &dir), e))?;
            fs::rename(p, &dest).map_err(|e| format!("Could not move '{}': {}", rel_of(ws, p), e))?;
        }
        moves.push(PhotoMove { from: rel_of(ws, p), to: rel_of(ws, &dest), date_from: source.into() });
    }
    let by_file = moves.iter().filter(|m| m.date_from == "file date").count();
    Ok(OrganizeResult {
        note: format!(
            "{} {} picture(s) into {} folder(s).{}",
            if dry_run { "Would move" } else { "Moved" },
            moves.len(),
            folders.len(),
            if by_file > 0 { format!(" {} had no 'date taken' in the photo, so the file's date was used.", by_file) } else { String::new() }
        ),
        moves,
        dry_run,
        folders: folders.into_iter().collect(),
    })
}

// ---------------------------------------------------------------
// remove_photo_location
// ---------------------------------------------------------------

/// Minimal EXIF block holding ONLY the rotation, so a stripped photo still
/// shows the right way up.
fn orientation_app1(orient: u32) -> Vec<u8> {
    let mut p = b"Exif\0\0II*\0".to_vec();
    p.extend_from_slice(&8u32.to_le_bytes()); // first IFD at offset 8
    p.extend_from_slice(&1u16.to_le_bytes()); // 1 entry
    p.extend_from_slice(&0x0112u16.to_le_bytes()); // Orientation
    p.extend_from_slice(&3u16.to_le_bytes()); // SHORT
    p.extend_from_slice(&1u32.to_le_bytes()); // count
    p.extend_from_slice(&(orient as u16).to_le_bytes());
    p.extend_from_slice(&[0, 0]); // padding to 4 bytes
    p.extend_from_slice(&0u32.to_le_bytes()); // no next IFD
    let mut seg = vec![0xFF, 0xE1];
    seg.extend_from_slice(&((p.len() + 2) as u16).to_be_bytes());
    seg.extend_from_slice(&p);
    seg
}

/// Drops APP1 (EXIF/XMP), APP13 (IPTC) and comments from a JPEG without touching
/// the compressed picture data.
pub fn strip_jpeg(data: &[u8], keep_orientation: u32) -> Result<Vec<u8>, String> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return Err("not a valid JPEG".into());
    }
    let mut out = vec![0xFF, 0xD8];
    let mut i = 2;
    let mut inserted = false;
    loop {
        if i + 2 > data.len() {
            return Err("JPEG ended early".into());
        }
        if data[i] != 0xFF {
            return Err("JPEG structure not understood".into());
        }
        let marker = data[i + 1];
        if marker == 0xFF {
            i += 1; // fill byte
            continue;
        }
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            out.extend_from_slice(&data[i..i + 2]);
            i += 2;
            continue;
        }
        if !inserted && marker != 0xE0 {
            if keep_orientation > 1 && keep_orientation <= 8 {
                out.extend_from_slice(&orientation_app1(keep_orientation));
            }
            inserted = true;
        }
        if marker == 0xDA || marker == 0xD9 {
            out.extend_from_slice(&data[i..]); // start of scan: the rest is picture data
            return Ok(out);
        }
        if i + 4 > data.len() {
            return Err("JPEG ended early".into());
        }
        let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        let end = i + 2 + len;
        if len < 2 || end > data.len() {
            return Err("JPEG segment length is wrong".into());
        }
        if !matches!(marker, 0xE1 | 0xED | 0xFE) {
            out.extend_from_slice(&data[i..end]);
        }
        i = end;
    }
}

/// Drops eXIf and text chunks (which can hold XMP with GPS) from a PNG.
pub fn strip_png(data: &[u8]) -> Result<Vec<u8>, String> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !data.starts_with(SIG) {
        return Err("not a valid PNG".into());
    }
    let mut out = SIG.to_vec();
    let mut i = SIG.len();
    while i + 12 <= data.len() {
        let len = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
        let end = i + 12 + len;
        if end > data.len() {
            return Err("PNG chunk length is wrong".into());
        }
        let kind = &data[i + 4..i + 8];
        if !matches!(kind, b"eXIf" | b"tEXt" | b"zTXt" | b"iTXt") {
            out.extend_from_slice(&data[i..end]);
        }
        i = end;
        if kind == b"IEND" {
            return Ok(out);
        }
    }
    Err("PNG ended early".into())
}

#[derive(Serialize, Debug)]
pub struct CleanedFile {
    pub path: String,
    pub had_location: bool,
    pub bytes_removed: u64,
}

#[derive(Serialize, Debug)]
pub struct CleanResult {
    pub cleaned: Vec<CleanedFile>,
    pub skipped: Vec<String>,
    pub note: String,
}

pub fn remove_photo_location(ws: &Workspace, relative_path: &str) -> Result<CleanResult, String> {
    let (files, _) = collect(ws, relative_path)?;
    let mut cleaned = Vec::new();
    let mut skipped = Vec::new();
    for p in &files {
        let rel = rel_of(ws, p);
        let data = match fs::read(p) {
            Ok(d) => d,
            Err(e) => {
                skipped.push(format!("{} ({})", rel, e));
                continue;
            }
        };
        let x = read_exif_bytes(&data);
        let had_location = x.as_ref().map(has_gps).unwrap_or(false);
        let stripped = match ext_of(p).as_str() {
            "jpg" | "jpeg" => strip_jpeg(&data, x.as_ref().map(orientation).unwrap_or(1)),
            "png" => strip_png(&data),
            other => {
                skipped.push(format!("{} (.{} is not supported; resize_image makes a clean JPEG/PNG copy)", rel, other));
                continue;
            }
        };
        let out = match stripped {
            Ok(o) => o,
            Err(e) => {
                skipped.push(format!("{} ({})", rel, e));
                continue;
            }
        };
        // confirm: still a readable picture, and no location left
        if image::ImageReader::new(Cursor::new(&out)).with_guessed_format().ok().and_then(|r| r.into_dimensions().ok()).is_none() {
            skipped.push(format!("{} (result would not be a readable picture; left unchanged)", rel));
            continue;
        }
        if read_exif_bytes(&out).as_ref().map(has_gps).unwrap_or(false) {
            skipped.push(format!("{} (location could not be removed; left unchanged)", rel));
            continue;
        }
        if out.len() != data.len() {
            let tmp = p.with_extension(format!("{}.aicw-tmp", ext_of(p)));
            let write = fs::File::create(&tmp).and_then(|mut f| f.write_all(&out).and_then(|_| f.sync_all()));
            if let Err(e) = write.and_then(|_| fs::rename(&tmp, p)) {
                let _ = fs::remove_file(&tmp);
                skipped.push(format!("{} (could not save: {})", rel, e));
                continue;
            }
        }
        cleaned.push(CleanedFile { path: rel, had_location, bytes_removed: (data.len() - out.len().min(data.len())) as u64 });
    }
    let n_loc = cleaned.iter().filter(|c| c.had_location).count();
    Ok(CleanResult {
        note: format!(
            "Cleaned {} picture(s) in place without re-compressing ({} had GPS location). Camera details, date taken and comments were removed too; rotation was kept.",
            cleaned.len(),
            n_loc
        ),
        cleaned,
        skipped,
    })
}

// ---------------------------------------------------------------
// image_thumbnail (Live view only, not an agent tool)
// ---------------------------------------------------------------

pub fn image_thumbnail(ws: &Workspace, relative_path: &str, size: u32) -> Result<String, String> {
    let p = ws.resolve(relative_path)?;
    if !p.is_file() || !DECODABLE_EXTS.contains(&ext_of(&p).as_str()) {
        return Err("not a picture that can be previewed".into());
    }
    let img = upright(decode(&p)?, read_exif_file(&p).as_ref().map(orientation).unwrap_or(1));
    let t = img.thumbnail(size.clamp(32, 512), size.clamp(32, 512));
    let mut buf = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 72).encode_image(&t.to_rgb8()).map_err(|e| e.to_string())?;
    Ok(format!("data:image/jpeg;base64,{}", base64(&buf)))
}

// ---------------------------------------------------------------
// Tests: `cargo test images`
// ---------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use exif::{Field, In, Tag, Value};

    fn ws(tag: &str) -> (PathBuf, Workspace) {
        let dir = std::env::temp_dir().join(format!("aicw_img_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let w = Workspace::new(dir.to_str().unwrap()).unwrap();
        (dir, w)
    }

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x * 7) as u8, (y * 5) as u8, 90]));
        let mut buf = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 90).encode_image(&img).unwrap();
        buf
    }

    /// A JPEG with a real EXIF block: date taken, camera, GPS, rotation 6.
    fn jpeg_with_exif(w: u32, h: u32) -> Vec<u8> {
        let fields = vec![
            Field { tag: Tag::Make, ifd_num: In::PRIMARY, value: Value::Ascii(vec![b"Pixel".to_vec()]) },
            Field { tag: Tag::Orientation, ifd_num: In::PRIMARY, value: Value::Short(vec![6]) },
            Field { tag: Tag::DateTimeOriginal, ifd_num: In::PRIMARY, value: Value::Ascii(vec![b"2019:07:14 10:22:33".to_vec()]) },
            Field { tag: Tag::GPSLatitudeRef, ifd_num: In::PRIMARY, value: Value::Ascii(vec![b"N".to_vec()]) },
            Field {
                tag: Tag::GPSLatitude,
                ifd_num: In::PRIMARY,
                value: Value::Rational(vec![(33, 1).into(), (41, 1).into(), (0, 1).into()]),
            },
        ];
        let mut w8 = exif::experimental::Writer::new();
        for f in &fields {
            w8.push_field(f);
        }
        let mut tiff = Cursor::new(Vec::new());
        w8.write(&mut tiff, false).unwrap();
        let mut payload = b"Exif\0\0".to_vec();
        payload.extend_from_slice(&tiff.into_inner());
        let base = jpeg(w, h);
        let mut out = base[..2].to_vec();
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&payload);
        out.extend_from_slice(&base[2..]);
        out
    }

    #[test]
    fn info_reports_location_but_not_coordinates() {
        let (dir, w) = ws("info");
        fs::write(dir.join("trip.jpg"), jpeg_with_exif(40, 20)).unwrap();
        fs::write(dir.join("plain.jpg"), jpeg(10, 10)).unwrap();
        fs::write(dir.join("notes.txt"), "x").unwrap();
        let r = image_info(&w, ".").unwrap();
        assert_eq!(r.count, 2, "only pictures");
        assert_eq!(r.with_location, 1);
        let trip = r.images.iter().find(|i| i.path == "trip.jpg").unwrap();
        assert_eq!((trip.width, trip.taken.as_deref(), trip.camera.as_deref()), (Some(40), Some("2019-07-14"), Some("Pixel")));
        assert!(r.note.contains("GPS"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn strips_location_losslessly_and_keeps_rotation() {
        let (dir, w) = ws("strip");
        let original = jpeg_with_exif(40, 20);
        fs::write(dir.join("trip.jpg"), &original).unwrap();
        let r = remove_photo_location(&w, "trip.jpg").unwrap();
        assert_eq!(r.cleaned.len(), 1);
        assert!(r.cleaned[0].had_location && r.cleaned[0].bytes_removed > 0);
        let after = fs::read(dir.join("trip.jpg")).unwrap();
        let x = read_exif_bytes(&after).expect("rotation block kept");
        assert!(!has_gps(&x) && exif_date(&x).is_none(), "GPS and date gone");
        assert_eq!(orientation(&x), 6, "rotation kept");
        // the compressed picture data is byte-for-byte the same (no re-encoding)
        let sos = |d: &[u8]| d.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
        assert_eq!(&after[sos(&after)..], &original[sos(&original)..]);
        assert_eq!(image::load_from_memory(&after).unwrap().width(), 40);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn png_text_and_exif_chunks_are_dropped() {
        let img = image::RgbImage::new(4, 4);
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(img).write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        // insert a tEXt chunk after IHDR (8 sig + 25 IHDR)
        let mut chunk = Vec::new();
        let body = b"Comment\0home: 33.68N";
        chunk.extend_from_slice(&(body.len() as u32).to_be_bytes());
        chunk.extend_from_slice(b"tEXt");
        chunk.extend_from_slice(body);
        chunk.extend_from_slice(&[0, 0, 0, 0]); // crc (not checked by our stripper)
        let mut with = png[..33].to_vec();
        with.extend_from_slice(&chunk);
        with.extend_from_slice(&png[33..]);
        let out = strip_png(&with).unwrap();
        assert_eq!(out, png);
        assert!(strip_png(b"nope").is_err() && strip_jpeg(b"nope", 1).is_err());
    }

    #[test]
    fn resize_makes_new_files_and_never_overwrites() {
        let (dir, w) = ws("resize");
        fs::write(dir.join("big.jpg"), jpeg_with_exif(400, 200)).unwrap();
        let r = resize_image(&w, "big.jpg", Some(100), None, None, None, None, Some("small".into())).unwrap();
        // EXIF rotation 6 -> upright copy is 200x400, then fit to width 100 -> 100x200
        assert_eq!((r.created[0].before.as_str(), r.created[0].after.as_str()), ("200x400", "100x200"));
        assert_eq!(r.created[0].to, "small/big-100x200.jpg");
        let copy = fs::read(dir.join("small/big-100x200.jpg")).unwrap();
        assert!(read_exif_bytes(&copy).map(|x| !has_gps(&x)).unwrap_or(true), "copies carry no GPS");
        let again = resize_image(&w, "big.jpg", Some(100), None, None, None, None, Some("small".into())).unwrap();
        assert_eq!(again.created[0].to, "small/big-100x200 (1).jpg");
        assert!(dir.join("big.jpg").exists(), "original untouched");
        let conv = resize_image(&w, "big.jpg", None, None, None, Some("png".into()), None, None).unwrap();
        assert_eq!(conv.created[0].to, "big-converted.png");
        assert!(resize_image(&w, "big.jpg", None, None, None, None, None, None).is_err(), "nothing to do");
        assert!(resize_image(&w, "big.jpg", None, None, None, Some("exe".into()), None, None).is_err());
        assert!(resize_image(&w, "../x.jpg", Some(10), None, None, None, None, None).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn target_size_fits_and_never_enlarges() {
        assert_eq!(target_size(4000, 3000, Some(1600), None, None), (1600, 1200));
        assert_eq!(target_size(4000, 3000, Some(1600), Some(900), None), (1200, 900));
        assert_eq!(target_size(800, 600, Some(1600), None, None), (800, 600));
        assert_eq!(target_size(800, 600, None, None, Some(50)), (400, 300));
    }

    #[test]
    fn organize_by_date_taken_then_file_date() {
        let (dir, w) = ws("org");
        fs::create_dir_all(dir.join("photos")).unwrap();
        fs::write(dir.join("photos/trip.jpg"), jpeg_with_exif(8, 8)).unwrap();
        fs::write(dir.join("photos/plain.jpg"), jpeg(8, 8)).unwrap();
        let plan = organize_photos(&w, "photos", "month", true).unwrap();
        assert!(plan.dry_run && dir.join("photos/trip.jpg").exists(), "dry run moves nothing");
        let r = organize_photos(&w, "photos", "month", false).unwrap();
        let trip = r.moves.iter().find(|m| m.from == "photos/trip.jpg").unwrap();
        assert_eq!((trip.to.as_str(), trip.date_from.as_str()), ("photos/2019/2019-07/trip.jpg", "photo"));
        assert!(dir.join("photos/2019/2019-07/trip.jpg").exists());
        let now = chrono::Local::now();
        let plain = r.moves.iter().find(|m| m.from == "photos/plain.jpg").unwrap();
        assert_eq!(plain.to, format!("photos/{}/{}/plain.jpg", now.format("%Y"), now.format("%Y-%m")));
        assert!(r.note.contains("file's date"));
        assert!(organize_photos(&w, "photos/2019/2019-07/trip.jpg", "month", false).is_err(), "folder only");
        assert!(organize_photos(&w, "photos", "week", false).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn thumbnail_is_a_data_url() {
        let (dir, w) = ws("thumb");
        fs::write(dir.join("a.jpg"), jpeg(300, 100)).unwrap();
        assert!(image_thumbnail(&w, "a.jpg", 96).unwrap().starts_with("data:image/jpeg;base64,/9j/"));
        assert!(image_thumbnail(&w, "../a.jpg", 96).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_folder_points_to_where_the_pictures_are() {
        let (dir, w) = ws("hint");
        fs::create_dir_all(dir.join("photos")).unwrap();
        fs::create_dir_all(dir.join("small")).unwrap();
        fs::write(dir.join("photos/a.jpg"), jpeg(8, 8)).unwrap();
        fs::write(dir.join("photos/b.jpg"), jpeg(8, 8)).unwrap();
        let e = resize_image(&w, "small", Some(100), None, None, None, None, None).unwrap_err();
        assert!(e.contains("no pictures") && e.contains("photos (2)"), "{}", e);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn exif_with_a_bad_pointer_is_still_read() {
        // written by Python's piexif: a strict parser rejects it ("Unexpected next IFD")
        let (dir, w) = ws("piexif");
        fs::write(dir.join("p.jpg"), include_bytes!("testdata/piexif-gps.jpg")).unwrap();
        let r = image_info(&w, "p.jpg").unwrap();
        assert!(r.images[0].has_location, "GPS found despite the malformed IFD pointer");
        assert!(remove_photo_location(&w, "p.jpg").unwrap().cleaned[0].had_location);
        assert!(!image_info(&w, "p.jpg").unwrap().images[0].has_location);
        let _ = fs::remove_dir_all(&dir);
    }
}
