use zip::ZipArchive;
use std::io::Read as IoRead;
use std::fs::File;

pub fn extract_docx_images(path: &std::path::Path) -> Result<Vec<Vec<u8>>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut images = Vec::new();

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if entry.name().starts_with("word/media/") {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).map_err(|e| e.to_string())?;
            images.push(buf);
        }
    }
    Ok(images)
}