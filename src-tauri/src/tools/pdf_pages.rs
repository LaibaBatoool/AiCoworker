//! merge_pdfs / extract_pdf_pages — page-level PDF editing inside the workspace.
//!
//! Safety/robustness properties:
//!  - every path goes through Workspace::resolve; inputs/outputs must be .pdf
//!  - the output file must be NEW (never overwrites; never one of the inputs)
//!  - encrypted PDFs are refused (we don't strip protection)
//!  - caps: 50 input files, 100 MB per input, 2000 pages per output
//!  - page order is preserved exactly; inherited page attributes
//!    (Resources, MediaBox, CropBox, Rotate) are copied onto each page so
//!    pages still render after they're moved under a new page tree
//!  - bookmarks, forms and other document-level extras are not carried over

use crate::workspace::Workspace;
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

const MAX_INPUTS: usize = 50;
const MAX_INPUT_BYTES: u64 = 100 * 1024 * 1024;
const MAX_OUTPUT_PAGES: usize = 2000;
const INHERITABLE: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

#[derive(Serialize, Debug)]
pub struct PdfPagesResult {
    pub output: String,
    pub pages_written: usize,
    /// For merges: pages taken from each input, in order.
    pub pages_per_input: Vec<usize>,
}

fn require_pdf(rel: &str) -> Result<(), String> {
    if rel.to_lowercase().ends_with(".pdf") {
        Ok(())
    } else {
        Err(format!("'{}' must be a .pdf file.", rel))
    }
}

fn load_pdf(ws: &Workspace, rel: &str) -> Result<(Document, PathBuf), String> {
    require_pdf(rel)?;
    let path = ws.resolve(rel)?;
    let meta = fs::metadata(&path).map_err(|_| format!("'{}' does not exist.", rel))?;
    if !meta.is_file() {
        return Err(format!("'{}' is not a file.", rel));
    }
    if meta.len() > MAX_INPUT_BYTES {
        return Err(format!("'{}' is larger than {} MB.", rel, MAX_INPUT_BYTES / (1024 * 1024)));
    }
    let doc = Document::load(&path).map_err(|e| format!("'{}' is not a readable PDF: {}", rel, e))?;
    if doc.is_encrypted() {
        return Err(format!("'{}' is encrypted/password-protected; refusing to modify it.", rel));
    }
    if doc.get_pages().is_empty() {
        return Err(format!("'{}' has no pages.", rel));
    }
    Ok((doc, path))
}

fn prepare_output(ws: &Workspace, rel: &str, inputs: &[PathBuf]) -> Result<PathBuf, String> {
    require_pdf(rel)?;
    let out = ws.resolve(rel)?;
    if out.exists() || inputs.iter().any(|p| p == &out) {
        return Err(format!("'{}' already exists. Choose a new output file name.", rel));
    }
    Ok(out)
}

/// A page's own dictionary plus any attributes it inherits from its
/// ancestors in the page tree (filled in only where the page lacks them).
fn page_with_inherited(doc: &Document, page_id: ObjectId) -> Result<Dictionary, String> {
    let mut dict = doc
        .get_dictionary(page_id)
        .map_err(|e| format!("Broken page object: {}", e))?
        .clone();
    let mut parent = dict.get(b"Parent").and_then(Object::as_reference).ok();
    let mut depth = 0;
    while let Some(pid) = parent {
        depth += 1;
        if depth > 64 {
            break; // malformed (cyclic) page tree
        }
        let Ok(node) = doc.get_dictionary(pid) else { break };
        for key in INHERITABLE {
            if !dict.has(key) {
                if let Ok(v) = node.get(key) {
                    dict.set(key.to_vec(), v.clone());
                }
            }
        }
        parent = node.get(b"Parent").and_then(Object::as_reference).ok();
    }
    Ok(dict)
}

fn save(doc: &mut Document, out: &PathBuf) -> Result<(), String> {
    doc.prune_objects();
    doc.renumber_objects();
    doc.save(out).map(|_| ()).map_err(|e| {
        let _ = fs::remove_file(out);
        format!("Failed to write PDF: {}", e)
    })
}

pub fn merge_pdfs(
    workspace: &Workspace,
    source_relative_paths: &[String],
    output_relative_path: &str,
) -> Result<PdfPagesResult, String> {
    if source_relative_paths.len() < 2 {
        return Err("Give at least two PDFs to merge.".into());
    }
    if source_relative_paths.len() > MAX_INPUTS {
        return Err(format!("At most {} PDFs can be merged at once.", MAX_INPUTS));
    }

    // Load everything first so a bad input fails before anything is written.
    let mut docs = Vec::new();
    let mut paths = Vec::new();
    for rel in source_relative_paths {
        let (d, p) = load_pdf(workspace, rel)?;
        docs.push(d);
        paths.push(p);
    }
    let out = prepare_output(workspace, output_relative_path, &paths)?;

    let mut merged = Document::with_version("1.7");
    let pages_id = merged.new_object_id();
    let mut next_id = merged.max_id + 1;
    let mut kids: Vec<Object> = Vec::new();
    let mut per_input = Vec::new();

    for mut doc in docs {
        doc.renumber_objects_with(next_id);
        next_id = doc.max_id + 1;

        // pages in reading order (get_pages is keyed by page number)
        let page_ids: Vec<ObjectId> = doc.get_pages().into_values().collect();
        let mut page_dicts = Vec::with_capacity(page_ids.len());
        for id in &page_ids {
            let mut d = page_with_inherited(&doc, *id)?;
            d.set("Parent", pages_id);
            page_dicts.push((*id, d));
        }
        per_input.push(page_ids.len());

        // copy every other object; old Catalog / page-tree nodes / outlines are dropped
        for (id, obj) in doc.objects.into_iter() {
            match obj.type_name().unwrap_or(b"") {
                b"Catalog" | b"Pages" | b"Page" | b"Outlines" | b"Outline" => {}
                _ => {
                    merged.objects.insert(id, obj);
                }
            }
        }
        for (id, d) in page_dicts {
            merged.objects.insert(id, Object::Dictionary(d));
            kids.push(Object::Reference(id));
        }
        if kids.len() > MAX_OUTPUT_PAGES {
            return Err(format!("The merged PDF would have more than {} pages.", MAX_OUTPUT_PAGES));
        }
    }

    let count = kids.len();
    merged.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count as i64 }),
    );
    merged.max_id = next_id;
    let catalog_id = merged.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    merged.trailer.set("Root", catalog_id);

    save(&mut merged, &out)?;
    Ok(PdfPagesResult { output: output_relative_path.to_string(), pages_written: count, pages_per_input: per_input })
}

/// Parses "1-3, 5, 8-" into page numbers (1-based), validated against `total`.
fn parse_pages(spec: &str, total: u32) -> Result<BTreeSet<u32>, String> {
    let mut set = BTreeSet::new();
    let bad = |part: &str| format!("Invalid page range '{}'. Use e.g. \"1-3,5,8-\" (this PDF has {} pages).", part, total);
    for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => {
                let a: u32 = if a.trim().is_empty() { 1 } else { a.trim().parse().map_err(|_| bad(part))? };
                let b: u32 = if b.trim().is_empty() { total } else { b.trim().parse().map_err(|_| bad(part))? };
                (a, b)
            }
            None => {
                let n: u32 = part.parse().map_err(|_| bad(part))?;
                (n, n)
            }
        };
        if a == 0 || b == 0 || a > b || b > total {
            return Err(bad(part));
        }
        set.extend(a..=b);
    }
    if set.is_empty() {
        return Err(bad(spec));
    }
    Ok(set)
}

pub fn extract_pdf_pages(
    workspace: &Workspace,
    relative_path: &str,
    pages: &str,
    output_relative_path: &str,
) -> Result<PdfPagesResult, String> {
    let (mut doc, path) = load_pdf(workspace, relative_path)?;
    let out = prepare_output(workspace, output_relative_path, &[path])?;
    let total = doc.get_pages().len() as u32;
    let keep = parse_pages(pages, total)?;
    if keep.len() > MAX_OUTPUT_PAGES {
        return Err(format!("At most {} pages can be extracted at once.", MAX_OUTPUT_PAGES));
    }
    let drop: Vec<u32> = (1..=total).filter(|n| !keep.contains(n)).collect();
    doc.delete_pages(&drop);
    // make every remaining page self-contained (inherited fonts/sizes copied
    // onto the page) so any reader — including simple text extractors — works
    for id in doc.get_pages().into_values().collect::<Vec<_>>() {
        let d = page_with_inherited(&doc, id)?;
        doc.objects.insert(id, Object::Dictionary(d));
    }
    save(&mut doc, &out)?;
    Ok(PdfPagesResult { output: output_relative_path.to_string(), pages_written: keep.len(), pages_per_input: vec![keep.len()] })
}

// ---------------------------------------------------------------
// Tests: `cargo test pdf_pages`
// ---------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::content::{Content, Operation};
    use lopdf::Stream;

    fn ws(tag: &str) -> (PathBuf, Workspace) {
        let base = std::env::temp_dir().join(format!("aicw_pdf_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let dir = base.join("ws");
        fs::create_dir_all(&dir).unwrap();
        let w = Workspace::new(dir.to_str().unwrap()).unwrap();
        (dir, w)
    }

    /// PDF with one page per label. Resources/MediaBox live on the PARENT
    /// node (inherited), which is the case that breaks naive merging.
    fn make_pdf(path: &std::path::Path, labels: &[&str]) {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier" });
        let mut kids = vec![];
        for l in labels {
            let content = Content {
                operations: vec![
                    Operation::new("BT", vec![]),
                    Operation::new("Tf", vec!["F1".into(), 24.into()]),
                    Operation::new("Td", vec![100.into(), 700.into()]),
                    Operation::new("Tj", vec![Object::string_literal(*l)]),
                    Operation::new("ET", vec![]),
                ],
            };
            let c = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
            kids.push(doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "Contents" => c }).into());
        }
        doc.objects.insert(pages_id, Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => kids, "Count" => labels.len() as i64,
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        }));
        let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", cat);
        doc.save(path).unwrap();
    }

    fn page_texts(path: &std::path::Path) -> Vec<String> {
        let doc = Document::load(path).unwrap();
        let n = doc.get_pages().len() as u32;
        (1..=n).map(|p| doc.extract_text(&[p]).unwrap().trim().to_string()).collect()
    }

    #[test]
    fn merge_keeps_order_and_inherited_resources() {
        let (dir, w) = ws("merge");
        make_pdf(&dir.join("a.pdf"), &["A1", "A2"]);
        make_pdf(&dir.join("b.pdf"), &["B1"]);
        make_pdf(&dir.join("c.pdf"), &["C1", "C2", "C3"]);
        let r = merge_pdfs(&w, &["c.pdf".into(), "a.pdf".into(), "b.pdf".into()], "all.pdf").unwrap();
        assert_eq!(r.pages_written, 6);
        assert_eq!(r.pages_per_input, vec![3, 2, 1]);
        assert_eq!(page_texts(&dir.join("all.pdf")), vec!["C1", "C2", "C3", "A1", "A2", "B1"]);
        // every page must carry its (inherited) font resources + MediaBox
        let doc = Document::load(dir.join("all.pdf")).unwrap();
        for id in doc.get_pages().values() {
            let d = doc.get_dictionary(*id).unwrap();
            assert!(d.has(b"Resources") && d.has(b"MediaBox"));
        }
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn extract_pages_by_range() {
        let (dir, w) = ws("extract");
        make_pdf(&dir.join("doc.pdf"), &["P1", "P2", "P3", "P4", "P5"]);
        let r = extract_pdf_pages(&w, "doc.pdf", "2-3, 5", "part.pdf").unwrap();
        assert_eq!(r.pages_written, 3);
        assert_eq!(page_texts(&dir.join("part.pdf")), vec!["P2", "P3", "P5"]);
        let r2 = extract_pdf_pages(&w, "doc.pdf", "4-", "tail.pdf").unwrap();
        assert_eq!(page_texts(&dir.join("tail.pdf")), vec!["P4", "P5"]);
        assert_eq!(r2.pages_written, 2);
        assert_eq!(Document::load(dir.join("doc.pdf")).unwrap().get_pages().len(), 5, "source is untouched");
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn bad_ranges_and_outputs_rejected() {
        let (dir, w) = ws("bad");
        make_pdf(&dir.join("doc.pdf"), &["P1", "P2"]);
        for spec in ["0", "3", "2-1", "1-9", "abc", "", " , "] {
            assert!(extract_pdf_pages(&w, "doc.pdf", spec, "x.pdf").is_err(), "should reject {:?}", spec);
        }
        assert!(extract_pdf_pages(&w, "doc.pdf", "1", "doc.pdf").is_err(), "must not overwrite the input");
        assert!(extract_pdf_pages(&w, "doc.pdf", "1", "out.txt").is_err(), "output must be .pdf");
        assert!(extract_pdf_pages(&w, "doc.pdf", "1", "../out.pdf").is_err(), "output must stay in workspace");
        assert!(merge_pdfs(&w, &["doc.pdf".into()], "m.pdf").is_err(), "need 2+ inputs");
        fs::write(dir.join("fake.pdf"), "not a pdf").unwrap();
        assert!(merge_pdfs(&w, &["doc.pdf".into(), "fake.pdf".into()], "m.pdf").is_err());
        assert!(!dir.join("m.pdf").exists(), "nothing written on failure");
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }
}