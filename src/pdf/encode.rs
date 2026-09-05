//! Unicode to WinAnsiEncoding (CP1252) transcoding, plus PDF literal-string
//! escaping. No font program is ever embedded, so every glyph the document can
//! show must survive this mapping.

/// What to do with a character that WinAnsi cannot represent.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Unmappable {
    /// Render it as a visible, reversible `\u{XXXX}` escape. Lossless.
    Escape,
    /// Substitute `?`. Smaller but lossy; opt-in only.
    Replace,
    /// Treat the file as an error.
    Fail,
}

/// CP1252 code points for bytes 0x80..=0x9F. `None` marks the five slots that
/// CP1252 leaves undefined (0x81, 0x8D, 0x8F, 0x90, 0x9D).
const HIGH: [Option<char>; 32] = [
    Some('\u{20AC}'), None,             Some('\u{201A}'), Some('\u{0192}'),
    Some('\u{201E}'), Some('\u{2026}'), Some('\u{2020}'), Some('\u{2021}'),
    Some('\u{02C6}'), Some('\u{2030}'), Some('\u{0160}'), Some('\u{2039}'),
    Some('\u{0152}'), None,             Some('\u{017D}'), None,
    None,             Some('\u{2018}'), Some('\u{2019}'), Some('\u{201C}'),
    Some('\u{201D}'), Some('\u{2022}'), Some('\u{2013}'), Some('\u{2014}'),
    Some('\u{02DC}'), Some('\u{2122}'), Some('\u{0161}'), Some('\u{203A}'),
    Some('\u{0153}'), None,             Some('\u{017E}'), Some('\u{0178}'),
];

/// The WinAnsi byte for `c`, or `None` if WinAnsi has no slot for it.
///
/// Control characters are deliberately unmappable: tabs are expanded before
/// this point and nothing else belongs in a text-showing operator.
pub fn winansi_byte(c: char) -> Option<u8> {
    match c {
        ' '..='~' => Some(c as u8),
        '\u{A0}'..='\u{FF}' => Some(c as u32 as u8),
        _ => HIGH.iter().position(|&h| h == Some(c)).map(|i| 0x80 + i as u8),
    }
}

/// Rewrite `line` so every character is WinAnsi-representable.
///
/// Returns the rewritten line and whether any character had to be escaped or
/// replaced. Under [`Unmappable::Fail`] returns the first offending character.
pub fn transcode_line(line: &str, mode: Unmappable) -> Result<(String, bool), char> {
    let mut out = String::with_capacity(line.len());
    let mut touched = false;
    for c in line.chars() {
        if winansi_byte(c).is_some() {
            out.push(c);
            continue;
        }
        touched = true;
        match mode {
            Unmappable::Escape => out.push_str(&format!("\\u{{{:X}}}", c as u32)),
            Unmappable::Replace => out.push('?'),
            Unmappable::Fail => return Err(c),
        }
    }
    Ok((out, touched))
}

/// Encode a line that has already been through [`transcode_line`].
pub fn encode_line(line: &str) -> Vec<u8> {
    line.chars().map(|c| winansi_byte(c).unwrap_or(b'?')).collect()
}

/// Escape a byte string for use inside a PDF literal string `( ... )`.
pub fn escape_literal(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 8);
    for &b in bytes {
        if matches!(b, b'(' | b')' | b'\\') {
            out.push(b'\\');
        }
        out.push(b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_maps_to_itself() {
        assert_eq!(winansi_byte('A'), Some(b'A'));
        assert_eq!(winansi_byte(' '), Some(b' '));
        assert_eq!(winansi_byte('~'), Some(b'~'));
    }

    #[test]
    fn latin1_range_maps_directly() {
        assert_eq!(winansi_byte('\u{A0}'), Some(0xA0));
        assert_eq!(winansi_byte('é'), Some(0xE9));
        assert_eq!(winansi_byte('ÿ'), Some(0xFF));
    }

    #[test]
    fn cp1252_high_range_maps_to_its_slot() {
        assert_eq!(winansi_byte('€'), Some(0x80));
        assert_eq!(winansi_byte('\u{2018}'), Some(0x91));
        assert_eq!(winansi_byte('\u{2019}'), Some(0x92));
        assert_eq!(winansi_byte('\u{201C}'), Some(0x93));
        assert_eq!(winansi_byte('\u{2014}'), Some(0x97));
        assert_eq!(winansi_byte('\u{2026}'), Some(0x85));
        assert_eq!(winansi_byte('\u{178}'), Some(0x9F));
    }

    #[test]
    fn undefined_cp1252_slots_and_controls_are_unmappable() {
        assert_eq!(winansi_byte('\u{81}'), None);
        assert_eq!(winansi_byte('\u{8D}'), None);
        assert_eq!(winansi_byte('\u{9D}'), None);
        assert_eq!(winansi_byte('\t'), None);
        assert_eq!(winansi_byte('\u{4E2D}'), None);
        assert_eq!(winansi_byte('\u{1F600}'), None);
    }

    #[test]
    fn escape_mode_is_visible_and_reversible() {
        let (out, escaped) = transcode_line("s = \"\u{1F600}\"", Unmappable::Escape).unwrap();
        assert_eq!(out, "s = \"\\u{1F600}\"");
        assert!(escaped);
    }

    #[test]
    fn replace_mode_substitutes_question_mark() {
        let (out, escaped) = transcode_line("a\u{4E2D}b", Unmappable::Replace).unwrap();
        assert_eq!(out, "a?b");
        assert!(escaped);
    }

    #[test]
    fn fail_mode_reports_the_offending_char() {
        assert_eq!(transcode_line("a\u{4E2D}b", Unmappable::Fail), Err('\u{4E2D}'));
    }

    #[test]
    fn mappable_line_is_unchanged_and_unflagged() {
        let (out, escaped) = transcode_line("  def f(x):  ", Unmappable::Escape).unwrap();
        assert_eq!(out, "  def f(x):  ");
        assert!(!escaped);
    }

    #[test]
    fn pdf_literal_escaping_covers_parens_and_backslash() {
        assert_eq!(escape_literal(br"a(b)c\d"), br"a\(b\)c\\d".to_vec());
    }

    #[test]
    fn encode_line_produces_winansi_bytes() {
        assert_eq!(encode_line("a\u{2014}b"), vec![b'a', 0x97, b'b']);
    }
}
