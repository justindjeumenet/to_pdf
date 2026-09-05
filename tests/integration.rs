//! Guards on the two goals: the PDFs must stay small, and the source must
//! survive the round trip unaltered.

use std::io::Read;
use std::path::PathBuf;
use std::process::Command;
use to_pdf::layout::{Geometry, paginate};
use to_pdf::pdf::encode::{Unmappable, transcode_line};
use to_pdf::pdf::{PdfOptions, build};

fn a4() -> Geometry {
    Geometry {
        width: 595.28,
        height: 841.89,
        margin: 36.0,
        font_size: 8.5,
        leading: 10.0,
    }
}

fn opts() -> PdfOptions {
    PdfOptions {
        width: 595.28,
        height: 841.89,
        margin: 36.0,
        font_size: 8.5,
        leading: 10.0,
        page_numbers: false,
    }
}

/// Full source -> PDF conversion, exactly as the pipeline does it.
fn convert(source: &str) -> Vec<u8> {
    let raw = to_pdf::extract::plain::extract(source.as_bytes());
    let lines: Vec<String> = raw
        .iter()
        .map(|l| transcode_line(l, Unmappable::Escape).unwrap().0)
        .collect();
    build(&paginate(&lines, &a4(), 4, false), &opts())
}

fn inflate(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(b)
        .read_to_end(&mut out)
        .unwrap();
    out
}

/// Every `stream ... endstream` payload, in file order. The scan only ever
/// looks for a new `stream` marker from outside the previous payload, so binary
/// flate bytes cannot produce a false match. Content streams are written first
/// and in page order, so page `n` is `streams(pdf)[n]`.
fn streams(pdf: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while let Some(p) = pdf[at..].windows(7).position(|w| w == b"stream\n") {
        let start = at + p + 7;
        let Some(e) = pdf[start..].windows(10).position(|w| w == b"\nendstream") else {
            break;
        };
        let end = start + e;
        out.push(pdf[start..end].to_vec());
        at = end + 10;
    }
    out
}

/// Recover the text lines of one page by undoing the content-stream encoding.
fn page_text(pdf: &[u8], page: usize) -> Vec<String> {
    let content = String::from_utf8_lossy(&inflate(&streams(pdf)[page])).into_owned();
    content
        .lines()
        .filter_map(|l| l.strip_prefix('(').and_then(|l| l.strip_suffix(")'")))
        .map(|l| {
            let mut out = String::with_capacity(l.len());
            let mut esc = false;
            for c in l.chars() {
                match (esc, c) {
                    (false, '\\') => esc = true,
                    _ => {
                        out.push(c);
                        esc = false;
                    }
                }
            }
            out
        })
        .collect()
}

// --- Guard 1: size budget -------------------------------------------------

#[test]
fn a_real_source_file_produces_a_pdf_smaller_than_itself() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/pdf/mod.rs");
    let source = std::fs::read_to_string(&path).unwrap();
    assert!(source.len() > 4000, "fixture must be a substantial file");
    let pdf = convert(&source);
    assert!(
        pdf.len() < source.len(),
        "PDF ({} B) must be smaller than the source ({} B)",
        pdf.len(),
        source.len()
    );
}

#[test]
fn a_tiny_file_does_not_pay_a_large_fixed_overhead() {
    let pdf = convert("fn main() {}\n");
    assert!(
        pdf.len() < 1200,
        "fixed overhead ballooned to {} B",
        pdf.len()
    );
}

#[test]
fn a_thousand_page_document_stays_dominated_by_its_text() {
    let source: String = (0..76_000)
        .map(|i| format!("let x{i} = compute({i});\n"))
        .collect();
    let pdf = convert(&source);
    assert!(
        pdf.len() < source.len() / 3,
        "1000 pages of repetitive text should compress hard, got {} B from {} B",
        pdf.len(),
        source.len()
    );
}

// --- Guard 2: determinism -------------------------------------------------

#[test]
fn conversion_is_byte_deterministic() {
    let source = "fn main() {\n    println!(\"hi\");\n}\n";
    assert_eq!(convert(source), convert(source));
}

// --- Guard 3: structural validity ----------------------------------------

#[test]
fn every_object_the_xref_names_is_resolvable() {
    let source: String = (0..300).map(|i| format!("line {i}\n")).collect();
    let pdf = convert(&source);

    assert!(pdf.starts_with(b"%PDF-1.5\n%"));
    assert!(pdf.ends_with(b"%%EOF\n"));

    let tail = String::from_utf8_lossy(&pdf[pdf.len().saturating_sub(64)..]).into_owned();
    let off: usize = tail
        .rsplit("startxref\n")
        .next()
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();

    let header = String::from_utf8_lossy(&pdf[off..(off + 200).min(pdf.len())]).into_owned();
    assert!(header.contains("/Type/XRef"), "got: {header}");
    assert!(header.contains("/W[1 4 2]"));
    assert!(header.contains("/Root 1 0 R"));
    let size: usize = header
        .split("/Size ")
        .nth(1)
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .trim()
        .parse()
        .unwrap();

    let table = {
        let start = off
            + pdf[off..]
                .windows(7)
                .position(|w| w == b"stream\n")
                .unwrap()
            + 7;
        let end = start
            + pdf[start..]
                .windows(10)
                .position(|w| w == b"\nendstream")
                .unwrap();
        inflate(&pdf[start..end])
    };
    assert_eq!(table.len(), size * 7, "/Size must match the entry count");

    let objstm_body = {
        let at = pdf.windows(12).position(|w| w == b"/Type/ObjStm").unwrap();
        let start = at + pdf[at..].windows(7).position(|w| w == b"stream\n").unwrap() + 7;
        let end = start
            + pdf[start..]
                .windows(10)
                .position(|w| w == b"\nendstream")
                .unwrap();
        String::from_utf8(inflate(&pdf[start..end])).unwrap()
    };
    let index: Vec<u32> = objstm_body
        .split_whitespace()
        .take_while(|t| t.chars().all(|c| c.is_ascii_digit()))
        .filter_map(|t| t.parse().ok())
        .collect();

    let mut in_stream = 0usize;
    for i in 0..size {
        let e = &table[i * 7..i * 7 + 7];
        let f2 = u32::from_be_bytes([e[1], e[2], e[3], e[4]]);
        let f3 = u16::from_be_bytes([e[5], e[6]]);
        match e[0] {
            0 => assert_eq!(i, 0, "only object 0 may be free"),
            1 => {
                let at = f2 as usize;
                let expected = format!("{i} 0 obj\n");
                assert!(
                    pdf[at..].starts_with(expected.as_bytes()),
                    "object {i} is not at its stated offset {at}"
                );
            }
            2 => {
                // The object stream's index pairs are `id offset`, so the id for
                // slot f3 sits at position 2 * f3.
                assert_eq!(
                    index[2 * f3 as usize],
                    i as u32,
                    "object {i} is not at index {f3} of stream {f2}"
                );
                in_stream += 1;
            }
            t => panic!("unknown xref entry type {t}"),
        }
    }
    assert!(
        in_stream >= 4,
        "catalog, pages, font and page dicts must be compressed"
    );
}

// --- Guard 4: source fidelity --------------------------------------------

#[test]
fn python_indentation_survives_the_round_trip_exactly() {
    let source = "\
class A:
    def f(self, x):
        if x > 0:
            return x
        else:

            return -x

    def g(self):
        return 0  
";
    let pdf = convert(source);
    let recovered = page_text(&pdf, 0);
    let expected: Vec<&str> = source.trim_end_matches('\n').split('\n').collect();
    assert_eq!(recovered.len(), expected.len());
    for (got, want) in recovered.iter().zip(&expected) {
        assert_eq!(
            got, want,
            "line differs, including leading and trailing space"
        );
    }
}

#[test]
fn parentheses_braces_and_backslashes_come_back_intact() {
    let source = r#"fn f(s: &str) -> String { format!("{}\\n", s) }"#;
    let pdf = convert(&format!("{source}\n"));
    assert_eq!(page_text(&pdf, 0), vec![source.to_string()]);
}

#[test]
fn unmappable_characters_are_escaped_visibly_not_silently_dropped() {
    let pdf = convert("s = \"\u{4E2D}\u{1F600}\"\n");
    assert_eq!(
        page_text(&pdf, 0),
        vec![r#"s = "\u{4E2D}\u{1F600}""#.to_string()],
        "unmappable chars must appear as visible escapes, never as ? or nothing"
    );
}

// --- End to end through the binary ---------------------------------------

#[test]
fn the_binary_converts_a_nested_tree_and_exits_zero() {
    let root = std::env::temp_dir().join("to_pdf_e2e");
    let _ = std::fs::remove_dir_all(&root);
    for (rel, body) in [
        ("src/main.rs", "fn main() {}\n"),
        ("src/deep/util.py", "def f():\n\treturn 1\n"),
        ("README.md", "# Title\n\ntext\n"),
        ("skipme.bin", "\x00\x01\x02"),
    ] {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }
    let out = root.join("out");

    let status = Command::new(env!("CARGO_BIN_EXE_to_pdf"))
        .arg(&root)
        .arg("-o")
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success());

    let base = out.join(root.file_name().unwrap());
    for rel in ["src/main.rs.pdf", "src/deep/util.py.pdf", "README.md.pdf"] {
        let p = base.join(rel);
        assert!(p.exists(), "missing {}", p.display());
        assert!(std::fs::read(&p).unwrap().starts_with(b"%PDF-1.5\n"));
    }
    assert!(!base.join("skipme.bin.pdf").exists());
}

#[test]
fn the_binary_exits_nonzero_when_a_file_fails() {
    let root = std::env::temp_dir().join("to_pdf_e2e_fail");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("broken.epub"), b"not a zip").unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_to_pdf"))
        .arg(&root)
        .arg("--in-place")
        .status()
        .unwrap();
    assert!(!status.success());
}
