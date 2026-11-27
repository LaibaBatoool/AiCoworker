//! generate_image: make a NEW picture from a text prompt (like Gemini / ChatGPT).
//!
//! Groq has no image models, so this uses a separate free service:
//!   - Cloudflare Workers AI, FLUX.1 [schnell] (recommended): free 10,000 "neurons"
//!     a day (~170 pictures at 1024x1024). Needs CLOUDFLARE_ACCOUNT_ID and
//!     CLOUDFLARE_API_TOKEN in src-tauri/.env.
//!   - Pollinations (fallback, no key, no signup): max 768x768, a small logo,
//!     about one picture per 45 s.
//! Chosen automatically (Cloudflare if both keys are set), or forced with
//! AICOWORKER_IMAGE_PROVIDER = cloudflare | pollinations | off.
//!
//! Safety: the prompt LEAVES the computer, so the tool is PRIVILEGED (approval),
//! and the prompt goes through the egress check first (no keys/secrets from the
//! workspace can be smuggled out inside a prompt). The answer must decode as a
//! real picture; it is re-encoded (no metadata kept) and saved inside the
//! workspace without ever overwriting a file.

use crate::workspace::Workspace;
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

const CF_MODEL: &str = "@cf/black-forest-labs/flux-1-schnell";
const CF_BASE: &str = "https://api.cloudflare.com/client/v4";
const POLL_BASE: &str = "https://image.pollinations.ai";
const MAX_PROMPT_CHARS: usize = 2000;
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub enum Provider {
    Cloudflare { base: String, account: String, token: String },
    Pollinations { base: String },
}

impl Provider {
    fn name(&self) -> &'static str {
        match self {
            Provider::Cloudflare { .. } => "Cloudflare Workers AI (FLUX.1 schnell)",
            Provider::Pollinations { .. } => "Pollinations (free, no key)",
        }
    }
}

#[derive(Serialize, Debug)]
pub struct GenResult {
    pub path: String,
    pub provider: String,
    pub width: u32,
    pub height: u32,
    pub size_bytes: u64,
    pub note: String,
}

fn env_val(k: &str) -> Option<String> {
    std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// Which service to use, from .env / environment.
pub fn choose(mode: Option<String>, account: Option<String>, token: Option<String>) -> Result<Provider, String> {
    let cf = match (account, token) {
        (Some(a), Some(t)) => Some(Provider::Cloudflare { base: CF_BASE.into(), account: a, token: t }),
        _ => None,
    };
    let poll = Provider::Pollinations { base: POLL_BASE.into() };
    match mode.map(|m| m.to_lowercase()).as_deref() {
        Some("off") => Err("Picture generation is turned off (AICOWORKER_IMAGE_PROVIDER=off).".into()),
        Some("pollinations") => Ok(poll),
        Some("cloudflare") => cf.ok_or_else(|| {
            "AICOWORKER_IMAGE_PROVIDER=cloudflare but CLOUDFLARE_ACCOUNT_ID / CLOUDFLARE_API_TOKEN are missing in src-tauri/.env.".to_string()
        }),
        _ => Ok(cf.unwrap_or(poll)),
    }
}

fn provider_from_env() -> Result<Provider, String> {
    choose(env_val("AICOWORKER_IMAGE_PROVIDER"), env_val("CLOUDFLARE_ACCOUNT_ID"), env_val("CLOUDFLARE_API_TOKEN"))
}

// ---------------------------------------------------------------- small helpers

fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        })
    };
    let body = s.split_once("base64,").map(|(_, b)| b).unwrap_or(s);
    let mut out = Vec::with_capacity(body.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in body.bytes() {
        if c == b'=' || c.is_ascii_whitespace() {
            continue;
        }
        let v = val(c).ok_or("the image service sent broken picture data")?;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// "a cute cat on the moon" -> "a-cute-cat-on-the-moon"
fn slug(prompt: &str) -> String {
    let words: Vec<String> = prompt
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(6)
        .map(|w| w.to_lowercase())
        .collect();
    let s: String = words.join("-").chars().take(40).collect();
    if s.is_empty() { "image".into() } else { s.trim_end_matches('-').to_string() }
}

/// `name.ext`, then `name (1).ext`, ... — never overwrites.
fn free_name(p: &Path) -> PathBuf {
    if !p.exists() {
        return p.to_path_buf();
    }
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("image").to_string();
    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("jpg").to_string();
    let dir = p.parent().map(|d| d.to_path_buf()).unwrap_or_default();
    (1..10_000).map(|i| dir.join(format!("{} ({}).{}", stem, i, ext))).find(|c| !c.exists()).unwrap_or(p.to_path_buf())
}

/// Create the folders of `rel` one level at a time; every level is boundary-checked
/// by Workspace::resolve first, so nothing can be created outside the workspace.
fn ensure_folders(ws: &Workspace, rel: &str) -> Result<(), String> {
    let parts: Vec<&str> = rel.split('/').filter(|p| !p.is_empty() && *p != ".").collect();
    let mut cur = String::new();
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        cur = if cur.is_empty() { part.to_string() } else { format!("{}/{}", cur, part) };
        let dir = ws.resolve(&cur)?;
        if !dir.exists() {
            std::fs::create_dir(&dir).map_err(|e| format!("Could not create the folder {}: {}", cur, e))?;
        } else if !dir.is_dir() {
            return Err(format!("{} exists but is not a folder", cur));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- providers

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

fn short(t: &str) -> String {
    t.chars().take(200).collect()
}

fn call_cloudflare(base: &str, account: &str, token: &str, prompt: &str, seed: Option<u64>) -> Result<Vec<u8>, String> {
    let url = format!("{}/accounts/{}/ai/run/{}", base.trim_end_matches('/'), url_encode(account), CF_MODEL);
    let mut body = json!({ "prompt": prompt, "steps": 4 });
    if let Some(s) = seed {
        body["seed"] = json!(s);
    }
    let resp = client()?
        .post(&url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .map_err(|e| format!("Could not reach Cloudflare Workers AI: {}", e))?;
    let status = resp.status().as_u16();
    let text = resp.text().unwrap_or_default();
    match status {
        200 => {
            let v: Value = serde_json::from_str(&text).map_err(|_| "Cloudflare sent an unreadable answer.".to_string())?;
            let b64 = v["result"]["image"].as_str().ok_or("Cloudflare's answer had no picture in it.")?;
            b64_decode(b64)
        }
        401 | 403 => Err("Cloudflare refused the API token: check CLOUDFLARE_API_TOKEN (it needs the 'Workers AI' permission) and CLOUDFLARE_ACCOUNT_ID in src-tauri/.env.".into()),
        429 => Err("Cloudflare's free daily allowance (10,000 neurons) is used up; it resets at 00:00 UTC (5 am Pakistan time).".into()),
        400 if text.to_lowercase().contains("nsfw") || text.to_lowercase().contains("flagged") => {
            Err("The image service refused this prompt (content filter). Try a different description.".into())
        }
        _ => Err(format!("Cloudflare image error (HTTP {}): {}", status, short(&text))),
    }
}

fn call_pollinations(base: &str, prompt: &str, width: u32, height: u32, seed: Option<u64>) -> Result<Vec<u8>, String> {
    let mut url = format!(
        "{}/prompt/{}?width={}&height={}&safe=true&nologo=true",
        base.trim_end_matches('/'),
        url_encode(prompt),
        width,
        height
    );
    if let Some(s) = seed {
        url.push_str(&format!("&seed={}", s));
    }
    let resp = client()?.get(&url).send().map_err(|e| format!("Could not reach Pollinations: {}", e))?;
    let status = resp.status().as_u16();
    let ctype = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    match status {
        200 if ctype.starts_with("image/") => {
            let bytes = resp.bytes().map_err(|e| format!("Pollinations download failed: {}", e))?;
            Ok(bytes.to_vec())
        }
        200 => Err("Pollinations did not send a picture.".into()),
        402 | 429 => Err("Pollinations' free no-key limit was hit (about one picture per 45 seconds). Wait a minute, or add the free Cloudflare keys to src-tauri/.env for ~170 pictures a day.".into()),
        _ => Err(format!("Pollinations error (HTTP {}): {}", status, short(&resp.text().unwrap_or_default()))),
    }
}

// ---------------------------------------------------------------- the tool

pub fn generate_image(
    ws: &Workspace,
    prompt: &str,
    relative_path: Option<&str>,
    size: Option<u32>,
    seed: Option<u64>,
) -> Result<GenResult, String> {
    let provider = provider_from_env()?;
    generate_off_runtime(provider, ws, prompt, relative_path, size, seed)
}

/// Blocking HTTP (reqwest::blocking) must NOT run on the agent's async runtime
/// thread: it panics ("Cannot drop a runtime in a context where blocking is not
/// allowed"). So the whole job runs on its own thread, like fetch_url and vision.
fn generate_off_runtime(
    provider: Provider,
    ws: &Workspace,
    prompt: &str,
    relative_path: Option<&str>,
    size: Option<u32>,
    seed: Option<u64>,
) -> Result<GenResult, String> {
    let ws = ws.clone();
    let prompt = prompt.to_string();
    let rel = relative_path.map(|r| r.to_string());
    std::thread::spawn(move || generate_with(&provider, &ws, &prompt, rel.as_deref(), size, seed))
        .join()
        .map_err(|_| "Picture generation crashed unexpectedly; nothing was saved.".to_string())?
}

pub fn generate_with(
    provider: &Provider,
    ws: &Workspace,
    prompt: &str,
    relative_path: Option<&str>,
    size: Option<u32>,
    seed: Option<u64>,
) -> Result<GenResult, String> {
    let prompt = prompt.trim();
    if prompt.chars().count() < 3 {
        return Err("Describe the picture in a few words (prompt is too short).".into());
    }
    if prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(format!("The prompt is too long (max {} characters).", MAX_PROMPT_CHARS));
    }
    // the prompt leaves the PC: never let it carry a secret from the workspace
    crate::egress::check_outgoing_text(ws.root(), "the image service", prompt)?;

    // where to save (checked BEFORE calling the service, so nothing is wasted)
    let rel = match relative_path.map(|r| r.trim()).filter(|r| !r.is_empty()) {
        Some(r) => r.replace('\\', "/"),
        None => format!("generated/{}.jpg", slug(prompt)),
    };
    ensure_folders(ws, &rel)?;
    let target = ws.resolve(&rel)?;
    let ext = target.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let png = match ext.as_str() {
        "png" => true,
        "jpg" | "jpeg" => false,
        _ => return Err("relative_path must end with .jpg, .jpeg or .png".into()),
    };

    let side = size.unwrap_or(1024).clamp(256, 1024);
    let bytes = match provider {
        Provider::Cloudflare { base, account, token } => call_cloudflare(base, account, token, prompt, seed)?,
        Provider::Pollinations { base } => {
            let s = side.min(768);
            call_pollinations(base, prompt, s, s, seed)?
        }
    };
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("The picture from the image service is too large.".into());
    }
    // must be a real picture; re-encoding drops any metadata the service added
    let img = image::load_from_memory(&bytes).map_err(|_| "The image service sent something that is not a picture.".to_string())?;
    let mut out = Vec::new();
    if png {
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    } else {
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92).encode_image(&img.to_rgb8()).map_err(|e| e.to_string())?;
    }
    let final_path = free_name(&target);
    std::fs::write(&final_path, &out).map_err(|e| format!("Could not save the picture: {}", e))?;
    let shown = final_path
        .strip_prefix(ws.root())
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or(rel.clone());
    let mut note = String::from("AI-generated picture; it can contain mistakes (hands, text).");
    if matches!(provider, Provider::Pollinations { .. }) {
        note.push_str(" Made with the free no-key service (max 768 px); add Cloudflare keys for 1024 px.");
    }
    if shown != rel {
        note.push_str(&format!(" {} already existed, so it was saved as {}.", rel, shown));
    }
    Ok(GenResult {
        path: shown,
        provider: provider.name().to_string(),
        width: img.width(),
        height: img.height(),
        size_bytes: out.len() as u64,
        note,
    })
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};
    use std::sync::{Arc, Mutex};

    fn tmp_ws(tag: &str) -> (Workspace, PathBuf) {
        let dir = std::env::temp_dir().join(format!("aicw_imggen_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (Workspace::new(dir.to_str().unwrap()).unwrap(), dir)
    }

    fn jpeg_bytes() -> Vec<u8> {
        let img = image::RgbImage::from_fn(64, 48, |x, y| image::Rgb([x as u8 * 4, y as u8 * 5, 120]));
        let mut v = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut v, 80).encode_image(&img).unwrap();
        v
    }

    fn b64(data: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut s = String::new();
        for ch in data.chunks(3) {
            let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
            for i in 0..4 {
                if i <= ch.len() { s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char) } else { s.push('=') }
            }
        }
        s
    }

    /// Tiny HTTP server: answers every request with (status, content-type, body); records request lines + bodies.
    fn server(status: u16, ctype: &'static str, body: Vec<u8>) -> (String, Arc<Mutex<Vec<String>>>) {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        std::thread::spawn(move || {
            for s in l.incoming().take(4) {
                let mut s = s.unwrap();
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = s.read(&mut chunk).unwrap_or(0);
                    if n == 0 { break; }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(h) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..h]).to_string();
                        let len: usize = head.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0))).unwrap_or(0);
                        if buf.len() >= h + 4 + len {
                            seen2.lock().unwrap().push(format!("{}\n{}", head, String::from_utf8_lossy(&buf[h + 4..h + 4 + len])));
                            break;
                        }
                    }
                }
                let head = format!("HTTP/1.1 {} X\r\ncontent-type: {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", status, ctype, body.len());
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&body);
            }
        });
        (url, seen)
    }

    #[test]
    fn imagegen_provider_choice() {
        let k = |s: &str| Some(s.to_string());
        assert!(matches!(choose(None, k("acc"), k("tok")), Ok(Provider::Cloudflare { .. })));
        assert!(matches!(choose(None, k("acc"), None), Ok(Provider::Pollinations { .. })));
        assert!(matches!(choose(k("Pollinations"), k("acc"), k("tok")), Ok(Provider::Pollinations { .. })));
        assert!(choose(k("cloudflare"), None, None).unwrap_err().contains("CLOUDFLARE_ACCOUNT_ID"));
        assert!(choose(k("off"), k("acc"), k("tok")).unwrap_err().contains("off"));
    }

    #[test]
    fn imagegen_cloudflare_saves_a_real_picture_without_overwriting() {
        let (ws, dir) = tmp_ws("cf");
        let body = format!("{{\"result\":{{\"image\":\"{}\"}},\"success\":true,\"errors\":[]}}", b64(&jpeg_bytes())).into_bytes();
        let (url, seen) = server(200, "application/json", body);
        let p = Provider::Cloudflare { base: url, account: "acc123".into(), token: "tok-xyz".into() };
        let r = generate_with(&p, &ws, "A cute cat on the moon, watercolor", None, None, Some(7)).unwrap();
        assert_eq!(r.path, "generated/a-cute-cat-on-the-moon.jpg");
        assert_eq!((r.width, r.height), (64, 48));
        assert!(image::open(dir.join(&r.path)).is_ok());
        let req = seen.lock().unwrap()[0].clone();
        assert!(req.starts_with("POST /accounts/acc123/ai/run/@cf/black-forest-labs/flux-1-schnell"), "{}", req);
        assert!(req.to_lowercase().contains("authorization: bearer tok-xyz"));
        assert!(req.contains("\"prompt\":\"A cute cat on the moon, watercolor\"") && req.contains("\"seed\":7"));
        // same name again -> a new file, the first one is untouched
        let r2 = generate_with(&p, &ws, "A cute cat on the moon, watercolor", Some("generated/a-cute-cat-on-the-moon.jpg"), None, None).unwrap();
        assert_eq!(r2.path, "generated/a-cute-cat-on-the-moon (1).jpg");
        assert!(r2.note.contains("already existed"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn imagegen_pollinations_png_and_errors() {
        let (ws, dir) = tmp_ws("poll");
        let (url, seen) = server(200, "image/jpeg", jpeg_bytes());
        let p = Provider::Pollinations { base: url };
        let r = generate_with(&p, &ws, "logo for AI CoWorker & friends", Some("art/2026/logo.png"), Some(1024), None).unwrap();
        assert_eq!(r.path, "art/2026/logo.png");
        assert_eq!(image::ImageFormat::from_path(dir.join("art/2026/logo.png")).unwrap(), image::ImageFormat::Png);
        assert!(image::open(dir.join("art/2026/logo.png")).is_ok());
        let req = seen.lock().unwrap()[0].clone();
        // prompt is URL-encoded and the size capped at 768 for the free service
        assert!(req.starts_with("GET /prompt/logo%20for%20AI%20CoWorker%20%26%20friends?width=768&height=768"), "{}", req);
        // rate limit and non-pictures give clear errors
        let (u2, _) = server(402, "application/json", b"{}".to_vec());
        assert!(generate_with(&Provider::Pollinations { base: u2 }, &ws, "a red apple", None, None, None).unwrap_err().contains("45 seconds"));
        let (u3, _) = server(200, "image/jpeg", b"<html>not a picture</html>".to_vec());
        assert!(generate_with(&Provider::Pollinations { base: u3 }, &ws, "a red apple", None, None, None).unwrap_err().contains("not a picture"));
        let (u4, _) = server(401, "application/json", b"{}".to_vec());
        let cf = Provider::Cloudflare { base: u4, account: "a".into(), token: "t".into() };
        assert!(generate_with(&cf, &ws, "a red apple", None, None, None).unwrap_err().contains("refused the API token"));
        let _ = std::fs::remove_dir_all(dir);
    }

        /// Regression (27 Nov live run): called from the agent's tokio runtime it panicked.
    #[test]
    fn imagegen_works_when_called_from_the_async_agent() {
        let (ws, dir) = tmp_ws("async");
        let (url, _) = server(200, "image/jpeg", jpeg_bytes());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let r = rt.block_on(async move {
            tokio::spawn(async move {
                generate_off_runtime(Provider::Pollinations { base: url }, &ws, "a red apple", None, None, None)
            })
            .await
            .unwrap()
        });
        assert_eq!(r.unwrap().path, "generated/a-red-apple.jpg");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn imagegen_boundaries_and_secrets_never_leave() {
        let (ws, dir) = tmp_ws("bound");
        std::fs::write(dir.join(".env"), "MY_SECRET=SuperSecretValue123\n").unwrap();
        // no server needed: every case must be refused BEFORE any request
        let p = Provider::Pollinations { base: "http://127.0.0.1:9".into() };
        assert!(generate_with(&p, &ws, "a cat", Some("../outside.jpg"), None, None).is_err());
        assert!(generate_with(&p, &ws, "a cat", Some("../evil/new/x.jpg"), None, None).is_err());
        assert!(!dir.parent().unwrap().join("evil").exists(), "no folder may be created outside");
        assert!(generate_with(&p, &ws, "a cat", Some("pic.gif"), None, None).unwrap_err().contains(".jpg"));
        assert!(generate_with(&p, &ws, "hi", None, None, None).unwrap_err().contains("too short"));
        assert!(generate_with(&p, &ws, &"x".repeat(2001), None, None, None).unwrap_err().contains("too long"));
        let e = generate_with(&p, &ws, "a poster that says SuperSecretValue123", None, None, None).unwrap_err();
        assert!(e.to_lowercase().contains("blocked") || e.contains(".env"), "{}", e);
        assert_eq!(slug("!!!"), "image");
        assert_eq!(b64_decode("data:image/png;base64,aGk=").unwrap(), b"hi");
        let _ = std::fs::remove_dir_all(dir);
    }
}