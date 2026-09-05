//! EPUB to plain text.
//!
//! ZIP reading uses the `zip` crate — hand-rolling central-directory parsing is
//! where the bugs live. The OPF is scanned with a minimal tag reader rather
//! than a full XML parser; the only shapes that matter are `<rootfile>`,
//! `<item>`, and `<itemref>`.

use std::io::{Cursor, Read, Seek};
use zip::ZipArchive;

pub fn extract(bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut zip = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| format!("not a readable EPUB archive: {e}"))?;

    let container = entry(&mut zip, "META-INF/container.xml")?;
    let opf_path = start_tags(&container, "rootfile")
        .iter()
        .find_map(|t| attr(t, "full-path"))
        .ok_or("container.xml declares no rootfile full-path")?;

    let opf = entry(&mut zip, &opf_path)?;
    let base = opf_path.rsplit_once('/').map_or(String::new(), |(d, _)| format!("{d}/"));

    let manifest: Vec<(String, String)> = start_tags(&opf, "item")
        .iter()
        .filter_map(|t| Some((attr(t, "id")?, attr(t, "href")?)))
        .collect();

    let mut lines: Vec<String> = Vec::new();
    for t in start_tags(&opf, "itemref") {
        let Some(idref) = attr(&t, "idref") else { continue };
        let Some((_, href)) = manifest.iter().find(|(id, _)| *id == idref) else { continue };
        let Ok(doc) = entry(&mut zip, &normalise(&format!("{base}{href}"))) else { continue };
        let chapter = super::html::extract(doc.as_bytes());
        if chapter.is_empty() {
            continue;
        }
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.extend(chapter);
    }

    if lines.is_empty() {
        return Err("EPUB spine produced no text".into());
    }
    Ok(lines)
}

fn entry<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str) -> Result<String, String> {
    let mut f = zip.by_name(name).map_err(|_| format!("missing EPUB entry: {name}"))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).map_err(|e| format!("unreadable EPUB entry {name}: {e}"))?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// The inner text of every `<name ...>` start tag, angle brackets excluded.
/// A tag whose name merely starts with `name` (`<itemref>` for `item`) is
/// skipped.
fn start_tags(xml: &str, name: &str) -> Vec<String> {
    let open = format!("<{name}");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(p) = rest.find(&open) {
        let after = &rest[p + open.len()..];
        if after.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            rest = after;
            continue;
        }
        match after.find('>') {
            Some(e) => {
                out.push(after[..e].to_string());
                rest = &after[e + 1..];
            }
            None => break,
        }
    }
    out
}

/// Value of `name="..."` or `name='...'` inside a start tag.
fn attr(tag: &str, name: &str) -> Option<String> {
    let key = format!("{name}=");
    let p = tag.find(&key)?;
    let rest = &tag[p + key.len()..];
    let q = rest.chars().next()?;
    if q != '"' && q != '\'' {
        return None;
    }
    let end = rest[1..].find(q)?;
    Some(rest[1..1 + end].to_string())
}

/// Collapse `.` and `..` segments in a ZIP-internal path.
fn normalise(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    fn epub(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut w = ZipWriter::new(Cursor::new(Vec::new()));
        let o = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, body) in entries {
            w.start_file(*name, o).unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    const CONTAINER: &str = r#"<?xml version="1.0"?>
<container><rootfiles>
<rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
</rootfiles></container>"#;

    const OPF: &str = r#"<?xml version="1.0"?>
<package><manifest>
<item id="c2" href="two.xhtml" media-type="application/xhtml+xml"/>
<item id="c1" href="one.xhtml" media-type="application/xhtml+xml"/>
<item id="cover" href="cover.png" media-type="image/png"/>
</manifest><spine>
<itemref idref="c1"/>
<itemref idref="c2"/>
</spine></package>"#;

    fn book() -> Vec<u8> {
        epub(&[
            ("mimetype", "application/epub+zip"),
            ("META-INF/container.xml", CONTAINER),
            ("OEBPS/content.opf", OPF),
            ("OEBPS/one.xhtml", "<html><body><h1>Chapter One</h1><p>alpha</p></body></html>"),
            ("OEBPS/two.xhtml", "<html><body><h1>Chapter Two</h1><p>beta</p></body></html>"),
        ])
    }

    #[test]
    fn chapters_come_out_in_spine_order_not_manifest_order() {
        assert_eq!(
            extract(&book()).unwrap(),
            vec!["Chapter One", "alpha", "", "Chapter Two", "beta"]
        );
    }

    #[test]
    fn hrefs_resolve_relative_to_the_opf_directory() {
        assert!(extract(&book()).unwrap().contains(&"alpha".to_string()));
    }

    #[test]
    fn dot_dot_segments_in_hrefs_are_normalised() {
        assert_eq!(normalise("OEBPS/../text/a.xhtml"), "text/a.xhtml");
        assert_eq!(normalise("OEBPS/./a.xhtml"), "OEBPS/a.xhtml");
        assert_eq!(normalise("a.xhtml"), "a.xhtml");
    }

    #[test]
    fn images_in_the_manifest_are_ignored() {
        let out = extract(&book()).unwrap();
        assert!(!out.iter().any(|l| l.contains("cover")));
    }

    #[test]
    fn a_missing_spine_document_is_skipped_not_fatal() {
        let b = epub(&[
            ("META-INF/container.xml", CONTAINER),
            ("OEBPS/content.opf", OPF),
            ("OEBPS/one.xhtml", "<p>alpha</p>"),
        ]);
        assert_eq!(extract(&b).unwrap(), vec!["alpha"]);
    }

    #[test]
    fn a_non_zip_input_is_a_clean_error() {
        assert!(extract(b"this is not a zip").is_err());
    }

    #[test]
    fn a_missing_container_is_a_clean_error() {
        let b = epub(&[("mimetype", "application/epub+zip")]);
        let e = extract(&b).unwrap_err();
        assert!(e.contains("container.xml"), "got: {e}");
    }

    #[test]
    fn an_empty_spine_is_a_clean_error() {
        let opf = r#"<package><manifest></manifest><spine></spine></package>"#;
        let b = epub(&[("META-INF/container.xml", CONTAINER), ("OEBPS/content.opf", opf)]);
        assert!(extract(&b).is_err());
    }

    #[test]
    fn itemref_does_not_match_the_item_tag_scan() {
        let tags = start_tags(OPF, "item");
        assert_eq!(tags.len(), 3, "must not pick up the two <itemref> tags");
    }

    #[test]
    fn attributes_parse_with_either_quote_style() {
        assert_eq!(attr(" id='x' href=\"y\"", "id").as_deref(), Some("x"));
        assert_eq!(attr(" id='x' href=\"y\"", "href").as_deref(), Some("y"));
        assert_eq!(attr(" id='x'", "missing"), None);
    }
}
