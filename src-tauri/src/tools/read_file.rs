use crate::workspace::Workspace;
use std::fs;
use std::io::Read;
use dotext::{Docx, MsDoc};
use tesseract::Tesseract;
use crate::tools::docx_images::extract_docx_images;
use crate::tools::pdf_images::{extract_pdf_images, ExtractedImage};

pub fn read_file(workspace: &Workspace, relative_path: &str) -> Result<String, String> {
    let resolved_path = workspace.resolve(relative_path)?;

    let extension = resolved_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match extension.as_str() {
        "pdf" => {
            // Regular text content
            let mut text = pdf_extract::extract_text(&resolved_path)
                .map_err(|e| format!("Failed to read PDF: {}", e))?;

            // Embedded images (JPEG or reconstructed raw), OCR'd with confidence filtering
            let images = extract_pdf_images(&resolved_path)?;
            for (i, image) in images.iter().enumerate() {
                let (temp_path, bytes): (std::path::PathBuf, &Vec<u8>) = match image {
                    ExtractedImage::Jpeg(bytes) => {
                        (std::env::temp_dir().join(format!("pdf_img_{}.jpg", i)), bytes)
                    }
                    ExtractedImage::RawPng(bytes) => {
                        (std::env::temp_dir().join(format!("pdf_img_{}.png", i)), bytes)
                    }
                };

                fs::write(&temp_path, bytes)
                    .map_err(|e| format!("Failed to write temp image: {}", e))?;

                if let Ok(ocr) = Tesseract::new(None, Some("eng")) {
                    if let Ok(mut loaded) = ocr.set_image(temp_path.to_str().unwrap_or("")) {
                        let confidence = loaded.mean_text_conf();
                        if confidence >= 40 {
                            if let Ok(ocr_text) = loaded.get_text() {
                                text.push_str(&format!("\n\n[PDF Image {} text]:\n{}", i + 1, ocr_text));
                            }
                        }
                    }
                }
                let _ = fs::remove_file(&temp_path); // cleanup, ignore errors
            }

            Ok(text)
        }
        "docx" => {
            // Regular text content
            let mut file = Docx::open(&resolved_path)
                .map_err(|e| format!("Failed to open DOCX: {}", e))?;
            let mut text = String::new();
            file.read_to_string(&mut text)
                .map_err(|e| format!("Failed to read DOCX content: {}", e))?;

            // Embedded images, OCR'd with confidence filtering
            let images = extract_docx_images(&resolved_path)?;
            for (i, image_bytes) in images.iter().enumerate() {
                let temp_path = std::env::temp_dir().join(format!("docx_img_{}.png", i));
                fs::write(&temp_path, image_bytes)
                    .map_err(|e| format!("Failed to write temp image: {}", e))?;

                if let Ok(ocr) = Tesseract::new(None, Some("eng")) {
                    if let Ok(mut loaded) = ocr.set_image(temp_path.to_str().unwrap_or("")) {
                        let confidence = loaded.mean_text_conf();
                        if confidence >= 40 {
                            if let Ok(ocr_text) = loaded.get_text() {
                                text.push_str(&format!("\n\n[Image {} text]:\n{}", i + 1, ocr_text));
                            }
                        }
                    }
                }
                let _ = fs::remove_file(&temp_path); // cleanup, ignore errors
            }

            Ok(text)
        }
        "png" | "jpg" | "jpeg" | "bmp" | "tiff" => {
            let mut ocr = Tesseract::new(None, Some("eng"))
                .map_err(|e| format!("Failed to init OCR engine: {}", e))?
                .set_image(resolved_path.to_str().ok_or("Invalid path encoding")?)
                .map_err(|e| format!("Failed to load image: {}", e))?;

            let confidence = ocr.mean_text_conf();
            let text = ocr.get_text().map_err(|e| format!("OCR failed: {}", e))?;

            if confidence < 40 {
                Ok(String::from("[No readable text detected in this image]"))
            } else {
                Ok(text)
            }
        }
        _ => fs::read_to_string(&resolved_path)
            .map_err(|e| format!("Failed to read file as text: {}", e)),
    }
}