//! Code and plain text. Per spec §5 the only transformation permitted here is
//! line-ending normalisation: no trimming, no collapsing, no reformatting.
//! Tab expansion belongs to the layout stage, which owns the column maths.

/// Split `bytes` into lines. Invalid UTF-8 becomes U+FFFD rather than an error.
pub fn extract(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let normalised = text.replace("\r\n", "\n").replace('\r', "\n");
    if normalised.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = normalised.split('\n').map(str::to_string).collect();
    // `split` gives a trailing empty element for text that ends in a newline;
    // that is a terminator, not a blank final line.
    if normalised.ends_with('\n') {
        lines.pop();
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_newlines_without_a_trailing_empty_line() {
        assert_eq!(extract(b"a\nb\n"), vec!["a", "b"]);
        assert_eq!(extract(b"a\nb"), vec!["a", "b"]);
    }

    #[test]
    fn crlf_and_lone_cr_normalise_to_lf() {
        assert_eq!(extract(b"a\r\nb\rc\n"), vec!["a", "b", "c"]);
    }

    #[test]
    fn indentation_and_trailing_space_are_byte_exact() {
        let src = "def f():\n    if x:\n        return 1  \n\n    return 0\n";
        assert_eq!(
            extract(src.as_bytes()),
            vec!["def f():", "    if x:", "        return 1  ", "", "    return 0"]
        );
    }

    #[test]
    fn tabs_are_left_for_the_layout_stage() {
        assert_eq!(extract(b"\tx"), vec!["\tx"]);
    }

    #[test]
    fn blank_lines_are_never_collapsed() {
        assert_eq!(extract(b"a\n\n\n\nb\n"), vec!["a", "", "", "", "b"]);
    }

    #[test]
    fn empty_input_yields_no_lines() {
        assert!(extract(b"").is_empty());
    }

    #[test]
    fn a_single_newline_yields_one_empty_line() {
        assert_eq!(extract(b"\n"), vec![""]);
    }

    #[test]
    fn invalid_utf8_is_replaced_not_rejected() {
        assert_eq!(extract(&[b'a', 0xFF, b'b']), vec!["a\u{FFFD}b"]);
    }
}
