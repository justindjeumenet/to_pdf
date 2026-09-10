//! Per-format text extraction. Every extractor returns plain lines and knows
//! nothing about pages, columns, or PDF.

pub mod html;
pub mod plain;

use std::path::Path;

/// Extensions converted when `--ext` is not given. Kept sorted so the set is
/// easy to scan and diff; a test enforces both sorting and uniqueness.
///
/// Deliberately excluded: binary formats (`png`, `svg`, `pdf`, `zip`) and bulk
/// data (`jsonl`, `lock` files), which either cannot be typeset as text or
/// produce enormous PDFs nobody reads. Pass `--ext` to include them anyway.
///
/// Entries are matched against a file's extension *or* its whole filename, so
/// `dockerfile` here catches both `Dockerfile` and `debug.Dockerfile`.
#[rustfmt::skip]
pub const DEFAULT_EXTS: &[&str] = &[
    "c",     "cc",    "cfg",   "cjs",   "cpp",   "cs",    "css",
    "csv",   "cts",   "dockerfile", "go",    "h",     "hpp",   "htm",
    "html",  "in",    "ini",   "ipynb", "java",  "js",    "json",
    "jsx",   "kt",    "lua",   "makefile", "markdown", "md",    "mjs",
    "mk",    "mts",   "php",   "py",    "qmd",   "rb",    "rs",
    "rst",   "scss",  "sh",    "sql",   "swift", "toml",  "ts",
    "tsx",   "txt",   "xml",   "yaml",  "yml",
];

/// Extract `bytes` according to `path`'s extension. Anything unrecognised is
/// treated as plain text, which is the safe default for a `--ext` override.
pub fn extract(path: &Path, bytes: &[u8]) -> Result<Vec<String>, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" => Ok(html::extract(bytes)),
        // Refused rather than skipped: reaching here means `--ext` named epub
        // explicitly, and typesetting the zip container as text would produce
        // pages of binary noise instead of a book.
        "epub" => Err("EPUB is not supported; convert it with an ebook tool first".into()),
        _ => Ok(plain::extract(bytes)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn dispatch_uses_the_extension_case_insensitively() {
        let out = extract(Path::new("a/b/Main.PY"), b"x = 1\n").unwrap();
        assert_eq!(out, vec!["x = 1"]);
    }

    #[test]
    fn epub_is_refused_even_when_ext_forces_it() {
        let e = extract(Path::new("book.epub"), b"PK\x03\x04").unwrap_err();
        assert!(e.contains("EPUB"), "got: {e}");
    }

    #[test]
    fn the_epub_refusal_is_case_insensitive() {
        assert!(extract(Path::new("BOOK.EPUB"), b"PK").is_err());
    }

    #[test]
    fn unknown_extensions_fall_back_to_plain_text() {
        let out = extract(Path::new("notes.conf"), b"k = v\n").unwrap();
        assert_eq!(out, vec!["k = v"]);
    }

    #[test]
    fn default_extension_set_covers_the_common_source_families() {
        for e in [
            // the originally specified ten
            "js", "ts", "go", "rs", "py", "txt", "md", "markdown", "html",
            // siblings that were silently missed
            "jsx", "tsx", "mjs", "cjs", "mts", "cts", "htm", "qmd", "rst", "css", "scss", "json",
            "yaml", "yml", "toml", "ini", "cfg", "sh", "sql", "xml", "lua", "rb", "java", "c", "h",
            "cpp", "hpp", "cs", "php", "swift", "kt",
        ] {
            assert!(DEFAULT_EXTS.contains(&e), "missing default extension: {e}");
        }
    }

    #[test]
    fn default_extension_set_is_sorted_and_free_of_duplicates() {
        let mut v = DEFAULT_EXTS.to_vec();
        v.sort_unstable();
        v.dedup();
        assert_eq!(
            v.len(),
            DEFAULT_EXTS.len(),
            "duplicate entry in DEFAULT_EXTS"
        );
        assert_eq!(v, DEFAULT_EXTS.to_vec(), "DEFAULT_EXTS must stay sorted");
    }

    #[test]
    fn binary_and_data_formats_are_never_defaults() {
        for e in [
            "png", "jpg", "gif", "svg", "pdf", "zip", "epub", "jsonl", "eval", "so", "dylib",
        ] {
            assert!(!DEFAULT_EXTS.contains(&e), "{e} must not be a default");
        }
    }

    #[test]
    fn htm_dispatches_to_the_html_extractor() {
        assert_eq!(extract(Path::new("a.htm"), b"<p>x</p>").unwrap(), vec!["x"]);
    }
}
