//! A minimal, size-first PDF 1.5 writer.
//!
//! Every decision here is made for output size: standard-14 Courier so no font
//! program is embedded, `/MediaBox` and `/Resources` inherited from the pages
//! node so page dicts stay ~40 bytes, all non-stream objects packed into one
//! object stream, a cross-reference stream instead of a 20-bytes-per-object
//! table, and one `'` operator per text line.

pub mod encode;
pub mod object;
pub mod xref;

use encode::{encode_line, escape_literal};
use object::{CompressedObj, flate, stream_object};
use xref::{Entry, object_stream, xref_data};

#[derive(Clone, Debug)]
pub struct PdfOptions {
    pub width: f64,
    pub height: f64,
    pub margin: f64,
    pub font_size: f64,
    pub leading: f64,
    pub page_numbers: bool,
}

/// Build the compressed content stream for one page.
fn content_stream(lines: &[String], page_no: usize, total: usize, o: &PdfOptions) -> Vec<u8> {
    let first_baseline = o.height - o.margin - o.font_size;
    let y0 = first_baseline + o.leading;

    let mut s: Vec<u8> = Vec::with_capacity(lines.iter().map(|l| l.len() + 4).sum::<usize>() + 96);
    s.extend_from_slice(
        format!(
            "BT\n/F0 {:.2} Tf\n{:.2} TL\n{:.2} {y0:.2} Td\n",
            o.font_size, o.leading, o.margin
        )
        .as_bytes(),
    );
    for l in lines {
        s.push(b'(');
        s.extend_from_slice(&escape_literal(&encode_line(l)));
        // `'` means "advance one line, then show" — one operator per line.
        s.extend_from_slice(b")'\n");
    }
    s.extend_from_slice(b"ET\n");

    if o.page_numbers {
        let label = format!("{page_no} / {total}");
        let w = label.chars().count() as f64 * o.font_size * 0.6;
        let x = (o.width - w) / 2.0;
        let y = o.margin / 2.0;
        s.extend_from_slice(format!("BT\n/F0 {:.2} Tf\n{x:.2} {y:.2} Td\n(", o.font_size).as_bytes());
        s.extend_from_slice(&escape_literal(&encode_line(&label)));
        s.extend_from_slice(b")Tj\nET\n");
    }

    flate(&s)
}

/// Serialize `pages` into a complete PDF file. An empty slice yields a
/// single blank page rather than a structurally invalid document.
pub fn build(pages: &[Vec<String>], o: &PdfOptions) -> Vec<u8> {
    let fallback: [Vec<String>; 1] = [Vec::new()];
    let pages: &[Vec<String>] = if pages.is_empty() { &fallback } else { pages };
    let n = pages.len();

    let page_id = |i: usize| 4 + i as u32;
    let content_id = |i: usize| 4 + n as u32 + i as u32;
    let objstm_id = 4 + 2 * n as u32;
    let xref_id = 5 + 2 * n as u32;

    let kids: String = (0..n).map(|i| format!("{} 0 R ", page_id(i))).collect();
    let mut compressed = vec![
        CompressedObj { id: 1, body: "<</Type/Catalog/Pages 2 0 R>>".into() },
        CompressedObj {
            id: 2,
            body: format!(
                "<</Type/Pages/Count {n}/Kids[{}]/MediaBox[0 0 {:.2} {:.2}]\
                 /Resources<</Font<</F0 3 0 R>>>>>>",
                kids.trim_end(),
                o.width,
                o.height
            ),
        },
        CompressedObj {
            id: 3,
            body: "<</Type/Font/Subtype/Type1/BaseFont/Courier/Encoding/WinAnsiEncoding>>".into(),
        },
    ];
    for i in 0..n {
        compressed.push(CompressedObj {
            id: page_id(i),
            body: format!("<</Type/Page/Parent 2 0 R/Contents {} 0 R>>", content_id(i)),
        });
    }

    let mut out: Vec<u8> = Vec::with_capacity(4096);
    out.extend_from_slice(b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n");

    // Object 0 is the free-list head, so the table is one longer than the
    // highest id.
    let mut entries = vec![Entry::Free; xref_id as usize + 1];

    for (i, page) in pages.iter().enumerate() {
        entries[content_id(i) as usize] = Entry::InFile(out.len() as u64);
        let data = content_stream(page, i + 1, n, o);
        out.extend_from_slice(&stream_object(content_id(i), "/Filter/FlateDecode", &data));
    }

    let (stm_dict, stm_data) = object_stream(&compressed);
    for (idx, c) in compressed.iter().enumerate() {
        entries[c.id as usize] = Entry::InStream { stm: objstm_id, idx: idx as u32 };
    }
    entries[objstm_id as usize] = Entry::InFile(out.len() as u64);
    out.extend_from_slice(&stream_object(objstm_id, &stm_dict, &stm_data));

    let xref_offset = out.len();
    entries[xref_id as usize] = Entry::InFile(xref_offset as u64);
    let xdict = format!(
        "/Type/XRef/Size {}/W[1 4 2]/Root 1 0 R/Filter/FlateDecode",
        entries.len()
    );
    out.extend_from_slice(&stream_object(xref_id, &xdict, &xref_data(&entries)));

    out.extend_from_slice(format!("startxref\n{xref_offset}\n%%EOF\n").as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn opts() -> PdfOptions {
        PdfOptions {
            width: 595.28, height: 841.89, margin: 36.0,
            font_size: 8.5, leading: 10.0, page_numbers: false,
        }
    }

    fn page(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    fn inflate(b: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        flate2::read::ZlibDecoder::new(b).read_to_end(&mut out).unwrap();
        out
    }

    /// Payload of the first `stream ... endstream` at or after byte `from`.
    fn stream_at(pdf: &[u8], from: usize) -> Vec<u8> {
        let start = from + pdf[from..].windows(7).position(|w| w == b"stream\n").unwrap() + 7;
        let end = start + pdf[start..].windows(10).position(|w| w == b"\nendstream").unwrap();
        pdf[start..end].to_vec()
    }

    /// Byte offset of the `N 0 obj` line introducing the object stream.
    fn find_objstm(pdf: &[u8]) -> usize {
        let at = pdf.windows(12).position(|w| w == b"/Type/ObjStm").unwrap();
        pdf[..at].windows(6).rposition(|w| w == b"0 obj\n").unwrap()
    }

    /// Every top-level object dictionary, concatenated. Excludes compressed
    /// stream payloads, whose binary bytes would make `contains` checks flaky.
    fn dict_headers(pdf: &[u8]) -> String {
        let text = String::from_utf8_lossy(pdf).into_owned();
        text.split("obj\n<<")
            .skip(1)
            .filter_map(|seg| seg.split(">>\nstream").next().map(str::to_string))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn startxref_offset(pdf: &[u8]) -> usize {
        let text = String::from_utf8_lossy(&pdf[pdf.len().saturating_sub(64)..]).into_owned();
        text.rsplit("startxref\n").next().unwrap()
            .split_whitespace().next().unwrap()
            .parse().unwrap()
    }

    #[test]
    fn header_and_trailer_are_well_formed() {
        let pdf = build(&[page(&["hello"])], &opts());
        assert!(pdf.starts_with(b"%PDF-1.5\n%"));
        assert!(pdf.ends_with(b"%%EOF\n"));
    }

    #[test]
    fn startxref_points_at_the_xref_object() {
        let pdf = build(&[page(&["a"]), page(&["b"])], &opts());
        let off = startxref_offset(&pdf);
        assert!(pdf[off..].starts_with(b"9 0 obj\n"));
        let head = String::from_utf8_lossy(&pdf[off..off + 120]).into_owned();
        assert!(head.contains("/Type/XRef"));
        assert!(head.contains("/W[1 4 2]"));
        assert!(head.contains("/Size 10"));
    }

    #[test]
    fn font_is_standard_14_courier_and_never_embedded() {
        let pdf = build(&[page(&["x"])], &opts());
        let s = String::from_utf8(inflate(&stream_at(&pdf, find_objstm(&pdf)))).unwrap();
        assert!(s.contains("/BaseFont/Courier"));
        assert!(s.contains("/Encoding/WinAnsiEncoding"));
        assert!(!s.contains("/FontFile"), "no font program may be embedded");
        assert!(!s.contains("/FontDescriptor"));
        assert!(!dict_headers(&pdf).contains("/FontFile"));
    }

    #[test]
    fn page_dicts_inherit_mediabox_and_resources_from_the_pages_node() {
        let pdf = build(&[page(&["a"]), page(&["b"])], &opts());
        let s = String::from_utf8(inflate(&stream_at(&pdf, find_objstm(&pdf)))).unwrap();
        assert!(s.contains("/MediaBox[0 0 595.28 841.89]"));
        assert!(s.contains("/Resources<</Font<</F0 3 0 R>>>>"));
        assert!(s.contains("<</Type/Page/Parent 2 0 R/Contents 6 0 R>>"));
        assert!(s.contains("<</Type/Page/Parent 2 0 R/Contents 7 0 R>>"));
        assert_eq!(s.matches("/MediaBox").count(), 1);
        assert_eq!(s.matches("/Resources").count(), 1);
    }

    #[test]
    fn each_text_line_costs_exactly_one_operator() {
        let pdf = build(&[page(&["one", "two", "three"])], &opts());
        let content = String::from_utf8(inflate(&stream_at(&pdf, 0))).unwrap();
        assert_eq!(content.matches(")'").count(), 3);
        assert!(!content.contains("Tj"), "use ' rather than T* + Tj");
        assert_eq!(content.matches(" Tf").count(), 1, "Tf is issued once per page");
        assert_eq!(content.matches(" TL").count(), 1);
        assert_eq!(content.matches(" Td").count(), 1);
    }

    #[test]
    fn first_baseline_sits_one_leading_below_the_td_point() {
        let pdf = build(&[page(&["x"])], &opts());
        let content = String::from_utf8(inflate(&stream_at(&pdf, 0))).unwrap();
        assert!(content.contains("36.00 807.39 Td"), "got: {content}");
    }

    #[test]
    fn parentheses_and_backslashes_in_source_are_escaped() {
        let pdf = build(&[page(&[r"fn f(x: &str) { \n }"])], &opts());
        let content = String::from_utf8(inflate(&stream_at(&pdf, 0))).unwrap();
        assert!(content.contains(r"(fn f\(x: &str\) { \\n })'"));
    }

    #[test]
    fn output_is_byte_deterministic() {
        let p = vec![page(&["alpha", "beta"]), page(&["gamma"])];
        assert_eq!(build(&p, &opts()), build(&p, &opts()));
    }

    #[test]
    fn no_info_xmp_or_id_is_emitted() {
        let pdf = build(&[page(&["x"])], &opts());
        let headers = dict_headers(&pdf);
        let objstm = String::from_utf8(inflate(&stream_at(&pdf, find_objstm(&pdf)))).unwrap();
        for hay in [&headers, &objstm] {
            assert!(!hay.contains("/Info"));
            assert!(!hay.contains("/ID"));
            assert!(!hay.contains("Metadata"));
        }
    }

    #[test]
    fn an_empty_document_still_produces_one_valid_page() {
        let pdf = build(&[], &opts());
        assert!(pdf.starts_with(b"%PDF-1.5\n%"));
        let s = String::from_utf8(inflate(&stream_at(&pdf, find_objstm(&pdf)))).unwrap();
        assert!(s.contains("/Count 1"));
    }

    #[test]
    fn page_numbers_use_a_separate_text_object() {
        let mut o = opts();
        o.page_numbers = true;
        let pdf = build(&[page(&["x"]), page(&["y"])], &o);
        let content = String::from_utf8(inflate(&stream_at(&pdf, 0))).unwrap();
        assert_eq!(content.matches("BT\n").count(), 2);
        assert!(content.contains("(1 / 2)Tj"));
    }
}
