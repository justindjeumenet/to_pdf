//! HTML to plain text.
//!
//! Hand-rolled rather than pulling in a parser: the output is reduced to bare
//! lines anyway, and the dependency budget is four crates. Tolerant of
//! malformed markup by construction — every unexpected shape degrades to
//! "emit the text, skip the tag".

/// Character entities worth decoding. Anything else passes through verbatim.
const NAMED: &[(&str, char)] = &[
    ("amp", '&'), ("lt", '<'), ("gt", '>'), ("quot", '"'), ("apos", '\''),
    ("nbsp", '\u{A0}'), ("mdash", '\u{2014}'), ("ndash", '\u{2013}'),
    ("hellip", '\u{2026}'), ("lsquo", '\u{2018}'), ("rsquo", '\u{2019}'),
    ("ldquo", '\u{201C}'), ("rdquo", '\u{201D}'), ("bull", '\u{2022}'),
    ("copy", '\u{A9}'), ("reg", '\u{AE}'), ("trade", '\u{2122}'),
    ("middot", '\u{B7}'), ("laquo", '\u{AB}'), ("raquo", '\u{BB}'),
    ("deg", '\u{B0}'), ("plusmn", '\u{B1}'), ("times", '\u{D7}'),
    ("divide", '\u{F7}'), ("eacute", '\u{E9}'), ("egrave", '\u{E8}'),
    ("agrave", '\u{E0}'), ("ccedil", '\u{E7}'), ("uuml", '\u{FC}'),
    ("ouml", '\u{F6}'), ("auml", '\u{E4}'), ("szlig", '\u{DF}'),
    ("ntilde", '\u{F1}'), ("aacute", '\u{E1}'), ("iacute", '\u{ED}'),
    ("oacute", '\u{F3}'), ("uacute", '\u{FA}'),
];

/// Tags that force a line break.
const BLOCK: &[&str] = &[
    "p", "div", "br", "hr", "li", "ul", "ol", "h1", "h2", "h3", "h4", "h5", "h6",
    "tr", "table", "section", "article", "blockquote", "header", "footer",
    "nav", "aside", "figure", "figcaption", "dl", "dt", "dd", "form", "main",
];

/// Tags contributing no text at all.
const DROPPED: &[&str] = &["img", "svg", "picture", "source", "iframe", "canvas"];

pub fn extract(bytes: &[u8]) -> Vec<String> {
    let src = String::from_utf8_lossy(bytes).into_owned();
    let b: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;
    let mut pre = 0usize;

    while i < b.len() {
        if b[i] == '<' {
            if b[i..].starts_with(&['<', '!', '-', '-']) {
                i = find(&b, i + 4, &['-', '-', '>']).map_or(b.len(), |p| p + 3);
                continue;
            }
            let Some(gt) = tag_end(&b, i) else { break };
            let raw: String = b[i + 1..gt].iter().collect();
            let closing = raw.starts_with('/');
            let name: String = raw
                .trim_start_matches('/')
                .chars()
                .take_while(char::is_ascii_alphanumeric)
                .collect::<String>()
                .to_ascii_lowercase();
            i = gt + 1;
            match name.as_str() {
                "script" | "style" | "head" if !closing => i = skip_element(&b, i, &name),
                "pre" => {
                    pre = if closing { pre.saturating_sub(1) } else { pre + 1 };
                    newline(&mut out);
                }
                "li" if !closing => {
                    newline(&mut out);
                    out.push_str("- ");
                }
                n if DROPPED.contains(&n) => {}
                n if BLOCK.contains(&n) => newline(&mut out),
                _ => {}
            }
            continue;
        }
        if b[i] == '&'
            && let Some((c, len)) = entity(&b, i)
        {
            push_char(&mut out, c, pre > 0);
            i += len;
            continue;
        }
        push_char(&mut out, b[i], pre > 0);
        i += 1;
    }
    finish(&out)
}

fn find(b: &[char], from: usize, pat: &[char]) -> Option<usize> {
    if b.len() < pat.len() {
        return None;
    }
    (from..=b.len() - pat.len()).find(|&i| b[i..i + pat.len()] == *pat)
}

/// Index of the `>` that closes the tag opening at `i`, honouring quoted
/// attribute values so `<p title="a > b">` is one tag, not two.
fn tag_end(b: &[char], i: usize) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (j, &c) in b.iter().enumerate().skip(i + 1) {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some(j),
            _ => {}
        }
    }
    None
}

/// Index just past the matching `</name>`, or end of input.
fn skip_element(b: &[char], from: usize, name: &str) -> usize {
    let close: Vec<char> = format!("</{name}").chars().collect();
    let n = close.len();
    let mut i = from;
    while i + n <= b.len() {
        if b[i..i + n].iter().zip(&close).all(|(a, c)| a.to_ascii_lowercase() == *c) {
            return tag_end(b, i).map_or(b.len(), |g| g + 1);
        }
        i += 1;
    }
    b.len()
}

fn newline(out: &mut String) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
}

fn push_char(out: &mut String, c: char, pre: bool) {
    if pre {
        out.push(c);
    } else if c.is_whitespace() {
        if !out.is_empty() && !out.ends_with(' ') && !out.ends_with('\n') {
            out.push(' ');
        }
    } else {
        out.push(c);
    }
}

/// Decode the entity starting at `i`, returning the character and how many
/// source characters it spanned.
fn entity(b: &[char], i: usize) -> Option<(char, usize)> {
    let semi = (i + 1..(i + 12).min(b.len())).find(|&j| b[j] == ';')?;
    let body: String = b[i + 1..semi].iter().collect();
    let len = semi - i + 1;
    if let Some(hex) = body.strip_prefix("#x").or_else(|| body.strip_prefix("#X")) {
        return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).map(|c| (c, len));
    }
    if let Some(dec) = body.strip_prefix('#') {
        return dec.parse::<u32>().ok().and_then(char::from_u32).map(|c| (c, len));
    }
    NAMED.iter().find(|(n, _)| *n == body).map(|&(_, c)| (c, len))
}

/// Trim trailing space, squeeze blank-line runs to one, drop leading and
/// trailing blanks.
fn finish(out: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for l in out.lines() {
        let l = l.trim_end();
        if l.is_empty() && lines.last().is_none_or(String::is_empty) {
            continue;
        }
        lines.push(l.to_string());
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(s: &str) -> Vec<String> {
        extract(s.as_bytes())
    }

    #[test]
    fn tags_are_stripped_and_text_kept() {
        assert_eq!(e("<p>Hello <b>world</b></p>"), vec!["Hello world"]);
    }

    #[test]
    fn block_tags_break_lines() {
        assert_eq!(e("<p>one</p><p>two</p>"), vec!["one", "two"]);
        assert_eq!(e("a<br>b"), vec!["a", "b"]);
    }

    #[test]
    fn script_style_and_head_contents_are_dropped() {
        assert_eq!(
            e("<head><title>T</title></head><body><script>var x = 1 < 2;</script><style>p{a:b}</style><p>kept</p></body>"),
            vec!["kept"]
        );
    }

    #[test]
    fn list_items_get_a_dash_prefix() {
        assert_eq!(e("<ul><li>one</li><li>two</li></ul>"), vec!["- one", "- two"]);
    }

    #[test]
    fn named_and_numeric_entities_decode() {
        assert_eq!(e("<p>a &amp; b &lt; c &#65; &#x42; &mdash; d</p>"), vec!["a & b < c A B \u{2014} d"]);
    }

    #[test]
    fn unknown_entities_pass_through_untouched() {
        assert_eq!(e("<p>&notreal; x</p>"), vec!["&notreal; x"]);
    }

    #[test]
    fn whitespace_collapses_outside_pre() {
        assert_eq!(e("<p>a   \n\t  b</p>"), vec!["a b"]);
    }

    #[test]
    fn pre_preserves_internal_whitespace() {
        assert_eq!(e("<pre>def f():\n    return 1</pre>"), vec!["def f():", "    return 1"]);
    }

    #[test]
    fn images_are_dropped_including_alt_text() {
        assert_eq!(e("<p>before<img src='x.png' alt='a picture'>after</p>"), vec!["beforeafter"]);
    }

    #[test]
    fn a_greater_than_inside_a_quoted_attribute_does_not_end_the_tag() {
        assert_eq!(e("<p title=\"a > b\">text</p>"), vec!["text"]);
    }

    #[test]
    fn comments_are_dropped() {
        assert_eq!(e("<p>a<!-- <p>hidden</p> -->b</p>"), vec!["ab"]);
    }

    #[test]
    fn an_unclosed_tag_at_eof_does_not_panic() {
        assert_eq!(e("<p>text</p><div class=\"x"), vec!["text"]);
    }

    #[test]
    fn runs_of_blank_lines_are_collapsed_to_one() {
        assert_eq!(e("<p>a</p><div></div><div></div><p>b</p>"), vec!["a", "b"]);
    }

    #[test]
    fn headings_are_kept_as_their_own_lines() {
        assert_eq!(e("<h1>Title</h1><p>body</p>"), vec!["Title", "body"]);
    }

    #[test]
    fn empty_input_yields_no_lines() {
        assert!(e("").is_empty());
    }
}
