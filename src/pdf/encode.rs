//! Unicode to WinAnsiEncoding (CP1252) transcoding, plus PDF literal-string
//! escaping. No font program is ever embedded, so every glyph the document can
//! show must survive this mapping.

/// What to do with a character that WinAnsi cannot represent.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Unmappable {
    /// Render it as a visible, reversible `\u{XXXX}` escape. Lossless.
    Escape,
    /// Spell it out in ASCII where an unambiguous spelling exists, and escape
    /// whatever has none. Meant for prose: `fi` beats both `?` and an escape.
    Fold,
    /// Substitute `?`. Smaller but lossy; opt-in only.
    Replace,
    /// Treat the file as an error.
    Fail,
}

/// CP1252 code points for bytes 0x80..=0x9F. `None` marks the five slots that
/// CP1252 leaves undefined (0x81, 0x8D, 0x8F, 0x90, 0x9D).
#[rustfmt::skip]
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
        _ => HIGH
            .iter()
            .position(|&h| h == Some(c))
            .map(|i| 0x80 + i as u8),
    }
}

/// ASCII spellings for non-CP1252 characters, sorted by code point.
///
/// Only unambiguous transliterations belong here. A character with no entry
/// falls back to a visible escape, so a reader can always tell an ASCII
/// spelling this table chose from text the encoder could not represent at all.
/// Zero-width characters map to nothing: EPUBs are full of them and they carry
/// nothing a fixed-pitch page can show.
#[rustfmt::skip]
const FOLD: &[(char, &str)] = &[
    ('\u{0132}', "IJ"),  ('\u{0133}', "ij"),
    // Every fixed-width space collapses to a plain one.
    ('\u{2000}', " "),   ('\u{2001}', " "),   ('\u{2002}', " "),
    ('\u{2003}', " "),   ('\u{2004}', " "),   ('\u{2005}', " "),
    ('\u{2006}', " "),   ('\u{2007}', " "),   ('\u{2008}', " "),
    ('\u{2009}', " "),   ('\u{200A}', " "),
    ('\u{200B}', ""),    ('\u{200C}', ""),    ('\u{200D}', ""),
    ('\u{2010}', "-"),   ('\u{2011}', "-"),   ('\u{2012}', "-"),
    ('\u{2015}', "--"),  ('\u{201B}', "'"),   ('\u{201F}', "\""),
    ('\u{2023}', "*"),   ('\u{2028}', " "),   ('\u{2029}', " "),
    ('\u{202F}', " "),   ('\u{2032}', "'"),   ('\u{2033}', "\""),
    ('\u{2034}', "'''"), ('\u{2043}', "-"),   ('\u{2044}', "/"),
    ('\u{205F}', " "),   ('\u{2060}', ""),
    ('\u{2153}', "1/3"), ('\u{2154}', "2/3"), ('\u{2155}', "1/5"),
    ('\u{2156}', "2/5"), ('\u{2157}', "3/5"), ('\u{2158}', "4/5"),
    ('\u{2159}', "1/6"), ('\u{215A}', "5/6"), ('\u{215B}', "1/8"),
    ('\u{215C}', "3/8"), ('\u{215D}', "5/8"), ('\u{215E}', "7/8"),
    ('\u{2190}', "<-"),  ('\u{2192}', "->"),  ('\u{2194}', "<->"),
    ('\u{21D0}', "<="),  ('\u{21D2}', "=>"),  ('\u{21D4}', "<=>"),
    ('\u{2212}', "-"),   ('\u{2215}', "/"),   ('\u{2248}', "~="),
    ('\u{2260}', "!="),  ('\u{2264}', "<="),  ('\u{2265}', ">="),
    ('\u{25AA}', "*"),   ('\u{25CF}', "*"),   ('\u{25E6}', "o"),
    ('\u{3000}', " "),
    // The Latin ligatures: the ones that cost whole words.
    ('\u{FB00}', "ff"),  ('\u{FB01}', "fi"),  ('\u{FB02}', "fl"),
    ('\u{FB03}', "ffi"), ('\u{FB04}', "ffl"), ('\u{FB05}', "ft"),
    ('\u{FB06}', "st"),  ('\u{FEFF}', ""),
];

/// The ASCII spelling of `c`, if [`FOLD`] has one.
fn fold(c: char) -> Option<&'static str> {
    FOLD.binary_search_by_key(&c, |(k, _)| *k)
        .ok()
        .map(|i| FOLD[i].1)
}

/// The visible, reversible spelling of a character WinAnsi has no slot for.
fn escaped(c: char) -> String {
    format!("\\u{{{:X}}}", c as u32)
}

/// Rewrite `line` so every character is WinAnsi-representable.
///
/// Returns the rewritten line and whether any character had to be escaped,
/// folded or replaced. Under [`Unmappable::Fail`] returns the first offending
/// character.
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
            Unmappable::Escape => out.push_str(&escaped(c)),
            Unmappable::Fold => match fold(c) {
                Some(ascii) => out.push_str(ascii),
                None => out.push_str(&escaped(c)),
            },
            Unmappable::Replace => out.push('?'),
            Unmappable::Fail => return Err(c),
        }
    }
    Ok((out, touched))
}

/// Encode a line that has already been through [`transcode_line`].
pub fn encode_line(line: &str) -> Vec<u8> {
    line.chars()
        .map(|c| winansi_byte(c).unwrap_or(b'?'))
        .collect()
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
    fn fold_spells_ligatures_out_in_ascii() {
        let (out, touched) =
            transcode_line("He could not \u{FB01}nd the \u{FB02}oor", Unmappable::Fold).unwrap();
        assert_eq!(out, "He could not find the floor");
        assert!(touched);
    }

    #[test]
    fn fold_drops_zero_width_characters_entirely() {
        let (out, _) = transcode_line("wo\u{200B}rd\u{FEFF}", Unmappable::Fold).unwrap();
        assert_eq!(out, "word");
    }

    #[test]
    fn fold_normalises_unicode_spaces_and_hyphens() {
        let (out, _) = transcode_line("re\u{2011}read\u{2009}now", Unmappable::Fold).unwrap();
        assert_eq!(out, "re-read now");
    }

    #[test]
    fn fold_falls_back_to_an_escape_when_it_has_no_spelling() {
        let (out, touched) = transcode_line("\u{4E2D}\u{1F600}", Unmappable::Fold).unwrap();
        assert_eq!(out, "\\u{4E2D}\\u{1F600}");
        assert!(touched);
    }

    #[test]
    fn fold_leaves_winansi_characters_untouched() {
        let line = "caf\u{E9} \u{201C}why\u{201D} \u{2014} 50\u{20AC}";
        let (out, touched) = transcode_line(line, Unmappable::Fold).unwrap();
        assert_eq!(out, line);
        assert!(!touched, "nothing here needs folding");
    }

    #[test]
    fn every_fold_replacement_is_representable_in_winansi() {
        for (from, to) in FOLD {
            assert!(
                winansi_byte(*from).is_none(),
                "U+{:04X} is already mappable and needs no fold",
                *from as u32
            );
            for c in to.chars() {
                assert!(
                    winansi_byte(c).is_some(),
                    "the fold for U+{:04X} emits unmappable {c:?}",
                    *from as u32
                );
            }
        }
    }

    #[test]
    fn the_fold_table_is_sorted_and_free_of_duplicates() {
        let keys: Vec<char> = FOLD.iter().map(|(c, _)| *c).collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(keys, sorted, "FOLD must stay sorted and duplicate-free");
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
        assert_eq!(
            transcode_line("a\u{4E2D}b", Unmappable::Fail),
            Err('\u{4E2D}')
        );
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
