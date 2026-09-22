use lopdf::{Document, Object};
use std::path::Path;
use image::{ImageBuffer, Rgb, Luma};

pub enum ExtractedImage {
    Jpeg(Vec<u8>),
    RawPng(Vec<u8>), // already-encoded PNG bytes, ready to write to disk
}

pub fn extract_pdf_images(path: &Path) -> Result<Vec<ExtractedImage>, String> {
    let doc = Document::load(path).map_err(|e| format!("Failed to load PDF: {}", e))?;
    let mut images = Vec::new();

    for (_id, object) in doc.objects.iter() {
        if let Object::Stream(stream) = object {
            let is_image = stream
                .dict
                .get(b"Subtype")
                .ok()
                .and_then(|o| o.as_name().ok())
                .map(|name| name == b"Image")
                .unwrap_or(false);

            if !is_image {
                continue;
            }

            let filter = stream
                .dict
                .get(b"Filter")
                .ok()
                .and_then(|o| o.as_name().ok());

            match filter {
                Some(f) if f == b"DCTDecode" => {
                    images.push(ExtractedImage::Jpeg(stream.content.clone()));
                }
                Some(f) if f == b"FlateDecode" => {
                    if let Some(png_bytes) = reconstruct_flate_image(stream) {
                        images.push(ExtractedImage::RawPng(png_bytes));
                    }
                }
                _ => {} // other/unknown filters: skip
            }
        }
    }

    Ok(images)
}

fn reconstruct_flate_image(stream: &lopdf::Stream) -> Option<Vec<u8>> {
    let width = stream.dict.get(b"Width").ok()?.as_i64().ok()? as u32;
    let height = stream.dict.get(b"Height").ok()?.as_i64().ok()? as u32;
    let bits = stream
        .dict
        .get(b"BitsPerComponent")
        .ok()
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(8);

    // We only handle the common case: 8-bit RGB or 8-bit Grayscale.
    // CMYK, indexed color, and other bit depths are out of scope for now.
    if bits != 8 {
        return None;
    }

    let raw = stream.decompressed_content().ok()?;
    let expected_rgb_len = (width * height * 3) as usize;
    let expected_gray_len = (width * height) as usize;

    let mut png_bytes: Vec<u8> = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut png_bytes);

    if raw.len() == expected_rgb_len {
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_raw(width, height, raw)?;
        img.write_to(&mut cursor, image::ImageFormat::Png).ok()?;
        Some(png_bytes)
    } else if raw.len() == expected_gray_len {
        let img: ImageBuffer<Luma<u8>, Vec<u8>> = ImageBuffer::from_raw(width, height, raw)?;
        img.write_to(&mut cursor, image::ImageFormat::Png).ok()?;
        Some(png_bytes)
    } else {
        None // unexpected size — likely a color space we don't handle (CMYK, indexed, etc.)
    }
}