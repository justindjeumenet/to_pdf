//! Turning a file's lines into fixed-size pages of fixed-width rows.
//!
//! Courier is fixed-pitch at 600/1000 em, so a column is exactly
//! `font_size * 0.6` points wide and no metrics table is needed.

const COURIER_ADVANCE: f64 = 0.6;

#[derive(Clone, Debug)]
pub struct Geometry {
    pub width: f64,
    pub height: f64,
    pub margin: f64,
    pub font_size: f64,
    pub leading: f64,
}

impl Geometry {
    /// Characters that fit between the margins.
    pub fn columns(&self) -> usize {
        let usable = self.width - 2.0 * self.margin;
        ((usable / (self.font_size * COURIER_ADVANCE)).floor()).max(1.0) as usize
    }

    /// Text rows that fit between the margins.
    pub fn rows(&self) -> usize {
        let usable = self.height - 2.0 * self.margin;
        ((usable / self.leading).floor()).max(1.0) as usize
    }
}

/// Replace tabs by advancing to the next tabstop. A flat N-space expansion
/// would break alignment for any tab that is not at a tabstop boundary.
pub fn expand_tabs(line: &str, tab_width: usize) -> String {
    if !line.contains('\t') {
        return line.to_string();
    }
    let w = tab_width.max(1);
    let mut out = String::with_capacity(line.len() + 8);
    let mut col = 0usize;
    for ch in line.chars() {
        if ch == '\t' {
            let n = w - (col % w);
            out.extend(std::iter::repeat_n(' ', n));
            col += n;
        } else {
            out.push(ch);
            col += 1;
        }
    }
    out
}

/// Expand, wrap, prefix the gutter, and chunk into pages.
///
/// The gutter is carved out of the column count rather than added to it, so
/// source column 0 lands at the same x-coordinate on every row of the page —
/// wrapped or not. Without that, a continuation indent would be
/// indistinguishable from real Python indentation.
pub fn paginate(
    lines: &[String],
    g: &Geometry,
    tab_width: usize,
    line_numbers: bool,
) -> Vec<Vec<String>> {
    if lines.is_empty() {
        return Vec::new();
    }
    let expanded: Vec<String> = lines.iter().map(|l| expand_tabs(l, tab_width)).collect();
    let cols = g.columns();
    let digits = expanded.len().to_string().len();

    let gutter = if line_numbers {
        digits + 2
    } else if expanded.iter().any(|l| l.chars().count() > cols) {
        2
    } else {
        0
    };
    let text_cols = cols.saturating_sub(gutter).max(1);

    let mut flat: Vec<String> = Vec::with_capacity(expanded.len());
    for (i, line) in expanded.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut start = 0usize;
        let mut first = true;
        loop {
            let end = (start + text_cols).min(chars.len());
            let seg: String = chars[start..end].iter().collect();
            flat.push(match (gutter > 0, line_numbers, first) {
                (false, _, _) => seg,
                (true, true, true) => format!("{:>digits$}  {seg}", i + 1),
                (true, true, false) => format!("{:>digits$}\u{BB} {seg}", ""),
                (true, false, true) => format!("  {seg}"),
                (true, false, false) => format!("\u{BB} {seg}"),
            });
            first = false;
            start = end;
            if start >= chars.len() {
                break;
            }
        }
    }

    flat.chunks(g.rows()).map(<[String]>::to_vec).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a4() -> Geometry {
        Geometry { width: 595.28, height: 841.89, margin: 36.0, font_size: 8.5, leading: 10.0 }
    }

    fn v(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a4_geometry_matches_the_spec() {
        assert_eq!(a4().columns(), 102);
        assert_eq!(a4().rows(), 76);
    }

    #[test]
    fn dense_geometry_fits_more() {
        let g = Geometry { font_size: 7.0, leading: 8.0, ..a4() };
        assert_eq!(g.columns(), 124);
        assert_eq!(g.rows(), 96);
    }

    #[test]
    fn tabs_advance_to_the_next_tabstop_not_a_flat_width() {
        assert_eq!(expand_tabs("\tx", 4), "    x");
        assert_eq!(expand_tabs("a\tb", 4), "a   b");
        assert_eq!(expand_tabs("abc\td", 4), "abc d");
        assert_eq!(expand_tabs("abcd\te", 4), "abcd    e");
        assert_eq!(expand_tabs("\t\tx", 4), "        x");
    }

    #[test]
    fn tab_width_is_honoured() {
        assert_eq!(expand_tabs("a\tb", 8), "a       b");
    }

    #[test]
    fn leading_and_trailing_whitespace_survive_untouched() {
        let pages = paginate(&v(&["    def f():  "]), &a4(), 4, false);
        assert_eq!(pages[0][0], "    def f():  ");
    }

    #[test]
    fn blank_lines_are_preserved_not_collapsed() {
        let pages = paginate(&v(&["a", "", "", "b"]), &a4(), 4, false);
        assert_eq!(pages[0], v(&["a", "", "", "b"]));
    }

    #[test]
    fn no_gutter_when_nothing_wraps() {
        let pages = paginate(&v(&["short"]), &a4(), 4, false);
        assert_eq!(pages[0][0], "short");
    }

    #[test]
    fn wrapping_uses_a_gutter_and_never_shifts_source_columns() {
        let long = "x".repeat(105);
        let pages = paginate(&v(&["    indented", &long]), &a4(), 4, false);
        assert_eq!(pages[0][0], "      indented", "normal lines get two blank gutter columns");
        assert_eq!(pages[0][1], format!("  {}", "x".repeat(100)));
        assert_eq!(pages[0][2], format!("\u{BB} {}", "x".repeat(5)));
    }

    #[test]
    fn wrap_boundary_is_exact() {
        let g = a4();
        let exactly = "y".repeat(g.columns());
        let over = "y".repeat(g.columns() + 1);
        assert_eq!(paginate(&v(&[&exactly]), &g, 4, false)[0].len(), 1, "exactly full must not wrap");
        assert_eq!(paginate(&v(&[&over]), &g, 4, false)[0].len(), 2);
    }

    #[test]
    fn line_numbers_force_a_gutter_and_align_right() {
        let pages = paginate(&v(&["a", "b"]), &a4(), 4, true);
        assert_eq!(pages[0][0], "1  a");
        assert_eq!(pages[0][1], "2  b");
    }

    #[test]
    fn line_number_width_follows_the_largest_number() {
        let lines: Vec<String> = (1..=120).map(|i| format!("l{i}")).collect();
        let pages = paginate(&lines, &a4(), 4, true);
        assert_eq!(pages[0][0], "  1  l1");
        assert_eq!(pages[1][0], format!("{:>3}  l77", 77));
    }

    #[test]
    fn line_numbered_continuations_show_the_wrap_marker_with_a_blank_number() {
        let long = "z".repeat(200);
        let pages = paginate(&v(&[&long]), &a4(), 4, true);
        assert_eq!(pages[0][0], format!("1  {}", "z".repeat(99)));
        assert_eq!(pages[0][1], format!(" \u{BB} {}", "z".repeat(99)));
    }

    #[test]
    fn pagination_chunks_by_row_count() {
        let lines: Vec<String> = (0..200).map(|i| i.to_string()).collect();
        let pages = paginate(&lines, &a4(), 4, false);
        assert_eq!(pages.len(), 3);
        assert_eq!(pages[0].len(), 76);
        assert_eq!(pages[1].len(), 76);
        assert_eq!(pages[2].len(), 48);
    }

    #[test]
    fn an_empty_line_produces_one_row_not_zero() {
        let pages = paginate(&v(&[""]), &a4(), 4, false);
        assert_eq!(pages[0], v(&[""]));
    }

    #[test]
    fn a_very_long_single_line_wraps_without_loss() {
        let long = "q".repeat(100_000);
        let pages = paginate(&v(&[&long]), &a4(), 4, false);
        let joined: String = pages
            .iter()
            .flatten()
            .map(|r| r.trim_start_matches("\u{BB} ").trim_start_matches("  "))
            .collect();
        assert_eq!(joined.len(), 100_000);
    }

    #[test]
    fn empty_input_yields_no_pages() {
        assert!(paginate(&[], &a4(), 4, false).is_empty());
    }
}
