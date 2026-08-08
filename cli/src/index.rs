//! Indexer: walk a directory tree, classify doc-like files, assign ids.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

/// Docs over this size stay listed but cannot be transferred (413).
pub const MAX_DOC_SIZE: u64 = 25 * 1024 * 1024;

const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "target",
    "node_modules",
    "dist",
    "build",
    ".venv",
    "vendor",
    ".next",
];

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Doc {
    pub id: u64,
    pub path: String,
    pub kind: String,
    pub size: u64,
    pub mtime: u64,
    #[serde(skip)]
    pub abs: PathBuf,
}

#[derive(Clone, Debug)]
pub struct Index {
    pub docs: Vec<Doc>,
    pub by_id: HashMap<u64, usize>,
}

pub fn index_dir(root: &Path) -> Result<Index, String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("cannot read {}: {e}", root.display()))?;
    let mut docs = Vec::new();
    walk(&root, &root, &mut docs);
    docs.sort_by(|a, b| a.path.cmp(&b.path));
    for (i, doc) in docs.iter_mut().enumerate() {
        doc.id = i as u64 + 1;
    }
    let by_id = docs
        .iter()
        .enumerate()
        .map(|(i, doc)| (doc.id, i))
        .collect();
    Ok(Index { docs, by_id })
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<Doc>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut items: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    items.sort_by_key(|e| e.file_name());
    for entry in items {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if ft.is_dir() {
            if SKIP_DIRS.contains(&name_str.as_ref()) {
                continue;
            }
            walk(root, &entry.path(), out);
        } else if ft.is_file() {
            let Some(kind) = kind_of(&name_str) else {
                continue;
            };
            let abs = entry.path();
            let rel = abs
                .strip_prefix(root)
                .unwrap_or(&abs)
                .to_string_lossy()
                .replace('\\', "/");
            let Ok(meta) = fs::metadata(&abs) else {
                continue;
            };
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            out.push(Doc {
                id: 0,
                path: rel,
                kind: kind.to_string(),
                size: meta.len(),
                mtime,
                abs,
            });
        }
    }
}

fn kind_of(name: &str) -> Option<&'static str> {
    let lower = name.to_lowercase();
    if let Some((base, ext)) = lower.rsplit_once('.') {
        if base.is_empty() {
            return None;
        }
        return match ext {
            "md" => Some("md"),
            "html" | "htm" => Some("html"),
            "rst" => Some("rst"),
            "adoc" | "asciidoc" => Some("adoc"),
            "txt" => Some("txt"),
            "pdf" => Some("pdf"),
            "docx" => Some("docx"),
            _ => None,
        };
    }
    if lower == "readme" {
        return Some("readme");
    }
    None
}

pub fn content_type(kind: &str) -> &'static str {
    match kind {
        "md" => "text/markdown",
        "html" => "text/html",
        "rst" => "text/x-rst",
        "adoc" => "text/x-asciidoc",
        "txt" => "text/plain",
        "readme" => "text/plain",
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        _ => "application/octet-stream",
    }
}

pub fn fmt_size(n: u64) -> String {
    if n >= 1024 * 1024 {
        format!("{:.1} MB", n as f64 / 1_048_576.0)
    } else {
        format!("{:.1} KB", n as f64 / 1024.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("docscrying-index-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("docs/nested")).unwrap();
        fs::create_dir_all(dir.join("target")).unwrap();
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::create_dir_all(dir.join("node_modules")).unwrap();
        fs::create_dir_all(dir.join("vendor")).unwrap();
        fs::create_dir_all(dir.join(".venv")).unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("README"), "no ext readme").unwrap();
        fs::write(dir.join("readme"), "lowercase readme").unwrap();
        fs::write(dir.join("README.md"), "# r").unwrap();
        fs::write(dir.join("docs/a.html"), "<html></html>").unwrap();
        fs::write(dir.join("docs/nested/b.rst"), "b").unwrap();
        fs::write(dir.join("docs/c.adoc"), "c").unwrap();
        fs::write(dir.join("docs/d.txt"), "d").unwrap();
        fs::write(dir.join("docs/e.pdf"), "e").unwrap();
        fs::write(dir.join("docs/f.docx"), "f").unwrap();
        fs::write(dir.join("docs/unknown.xyz"), "x").unwrap();
        fs::write(dir.join("docs/.env"), "skip").unwrap();
        fs::write(dir.join("target/g.md"), "skip me").unwrap();
        fs::write(dir.join(".git/h.md"), "skip me").unwrap();
        fs::write(dir.join("node_modules/i.md"), "skip me").unwrap();
        fs::write(dir.join("vendor/j.md"), "skip me").unwrap();
        fs::write(dir.join(".venv/k.md"), "skip me").unwrap();
        fs::write(dir.join("src/README"), "nested readme").unwrap();
        dir
    }

    #[test]
    fn indexes_known_kinds_sorted_sequential_ids() {
        let dir = corpus();
        let index = index_dir(&dir).unwrap();
        let paths: Vec<&str> = index.docs.iter().map(|d| d.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "README",
                "README.md",
                "docs/a.html",
                "docs/c.adoc",
                "docs/d.txt",
                "docs/e.pdf",
                "docs/f.docx",
                "docs/nested/b.rst",
                "readme",
                "src/README",
            ]
        );
        let kinds: Vec<&str> = index.docs.iter().map(|d| d.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["readme", "md", "html", "adoc", "txt", "pdf", "docx", "rst", "readme", "readme"]
        );
        for (i, doc) in index.docs.iter().enumerate() {
            assert_eq!(doc.id, i as u64 + 1, "sequential ids");
            assert!(doc.mtime > 0, "mtime in unix seconds");
            assert!(doc.abs.is_absolute(), "absolute read path");
        }
        // every listed file exists on disk and sizes match
        for doc in &index.docs {
            assert_eq!(fs::read(&doc.abs).unwrap().len() as u64, doc.size);
        }
    }

    #[test]
    fn oversized_docs_stay_listed() {
        let dir = corpus();
        let big = dir.join("docs/big.md");
        fs::write(&big, vec![b'x'; (MAX_DOC_SIZE + 1) as usize]).unwrap();
        let index = index_dir(&dir).unwrap();
        let big_doc = index
            .docs
            .iter()
            .find(|d| d.path == "docs/big.md")
            .expect("oversized doc listed");
        assert!(big_doc.size > MAX_DOC_SIZE);
        let _ = fs::remove_file(&big);
    }

    #[test]
    fn content_types() {
        assert_eq!(content_type("md"), "text/markdown");
        assert_eq!(content_type("html"), "text/html");
        assert_eq!(content_type("rst"), "text/x-rst");
        assert_eq!(content_type("adoc"), "text/x-asciidoc");
        assert_eq!(content_type("txt"), "text/plain");
        assert_eq!(content_type("readme"), "text/plain");
        assert_eq!(content_type("pdf"), "application/pdf");
        assert_eq!(
            content_type("docx"),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        );
        assert_eq!(content_type("nope"), "application/octet-stream");
    }
}
