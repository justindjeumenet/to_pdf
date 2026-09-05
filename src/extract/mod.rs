//! Per-format text extraction. Every extractor returns plain lines and knows
//! nothing about pages, columns, or PDF.

pub mod epub;
pub mod html;
pub mod plain;

use std::path::Path;

/// Extensions converted when `--ext` is not given.
pub const DEFAULT_EXTS: &[&str] = &[
    "js", "ts", "go", "rs", "py", "txt", "md", "markdown", "html", "epub",
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
        "html" => Ok(html::extract(bytes)),
        "epub" => epub::extract(bytes),
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
    fn unknown_extensions_fall_back_to_plain_text() {
        let out = extract(Path::new("notes.conf"), b"k = v\n").unwrap();
        assert_eq!(out, vec!["k = v"]);
    }

    #[test]
    fn default_extension_set_matches_the_spec() {
        assert_eq!(
            DEFAULT_EXTS,
            ["js", "ts", "go", "rs", "py", "txt", "md", "markdown", "html", "epub"]
        );
    }
}
