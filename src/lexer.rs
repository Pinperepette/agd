//! Line classifier for AGD source.
//!
//! Stateless: each line is tagged independently. The parser owns fence
//! state and reinterprets `BlockStart` lines as `Continuation` while
//! inside a fenced region.

use crate::ast::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// `@<tag> ...` — line starts with `@` followed by a tag-name char
    BlockStart,
    /// `- <text>` — list item (only meaningful within `@ul` / `@ol`)
    ListItem,
    /// `> <text>` — quote line (only meaningful within `@quote`)
    QuoteLine,
    /// `~~~` — fence delimiter
    Fence,
    /// `@!<text>` — comment line
    Comment,
    /// blank line — terminates open multi-line block scope
    Empty,
    /// any other line — verbatim continuation (used inside fences)
    Continuation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line {
    pub kind: LineKind,
    pub span: Span,
    pub line_no: u32,
}

impl Line {
    pub fn slice<'a>(&self, src: &'a str) -> &'a str {
        &src[self.span.range()]
    }
}

/// Classify the source into a stream of `Line` records. Span ranges
/// exclude the terminating newline. Files without a trailing newline
/// emit a final `Line` whose span ends at the last byte.
pub fn tokenize(src: &str) -> Vec<Line> {
    let mut out = Vec::with_capacity(src.len() / 32 + 1);
    let mut line_no: u32 = 1;
    let mut cursor = 0usize;
    for raw in src.split_inclusive('\n') {
        let has_nl = raw.ends_with('\n');
        let body_len = if has_nl { raw.len() - 1 } else { raw.len() };
        let body = &raw[..body_len];
        let span = Span::new(cursor, cursor + body_len);
        let kind = classify(body);
        out.push(Line { kind, span, line_no });
        cursor += raw.len();
        line_no += 1;
    }
    // Edge: empty input → no lines. This is a valid empty document.
    out
}

fn classify(s: &str) -> LineKind {
    if s.is_empty() {
        return LineKind::Empty;
    }
    if is_fence_line(s) {
        return LineKind::Fence;
    }
    let bytes = s.as_bytes();
    if bytes[0] == b'@' {
        if bytes.len() >= 2 && bytes[1] == b'!' {
            return LineKind::Comment;
        }
        if bytes.len() >= 2 && is_tag_first(bytes[1]) {
            return LineKind::BlockStart;
        }
    }
    if s.starts_with("- ") || s == "-" {
        return LineKind::ListItem;
    }
    if s.starts_with("> ") || s == ">" {
        return LineKind::QuoteLine;
    }
    LineKind::Continuation
}

#[inline]
fn is_tag_first(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

/// A fence line is a run of `~` of length ≥ 3, with no other characters.
/// Variable length lets a body containing `~~~` be wrapped by `~~~~` (or more).
#[inline]
pub(crate) fn is_fence_line(s: &str) -> bool {
    s.len() >= 3 && s.bytes().all(|b| b == b'~')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<LineKind> {
        tokenize(src).into_iter().map(|l| l.kind).collect()
    }

    #[test]
    fn empty_input_yields_no_lines() {
        assert!(tokenize("").is_empty());
    }

    #[test]
    fn classifies_basic_blocks() {
        let src = "@h1 Hello\n@p World\n";
        assert_eq!(kinds(src), vec![LineKind::BlockStart, LineKind::BlockStart]);
    }

    #[test]
    fn empty_line_separates_blocks() {
        let src = "@p A\n\n@p B\n";
        assert_eq!(
            kinds(src),
            vec![
                LineKind::BlockStart,
                LineKind::Empty,
                LineKind::BlockStart,
            ]
        );
    }

    #[test]
    fn list_and_quote_markers() {
        let src = "@ul\n- one\n- two\n@quote\n> q\n";
        assert_eq!(
            kinds(src),
            vec![
                LineKind::BlockStart,
                LineKind::ListItem,
                LineKind::ListItem,
                LineKind::BlockStart,
                LineKind::QuoteLine,
            ]
        );
    }

    #[test]
    fn fences_and_continuation() {
        let src = "@code\n~~~\nfn main() {}\n~~~\n";
        assert_eq!(
            kinds(src),
            vec![
                LineKind::BlockStart,
                LineKind::Fence,
                LineKind::Continuation,
                LineKind::Fence,
            ]
        );
    }

    #[test]
    fn longer_fences_recognised() {
        // Lines composed of ≥3 tildes are all classified as Fence.
        for n in 3..=8 {
            let s = "~".repeat(n);
            assert_eq!(kinds(&format!("{}\n", s)), vec![LineKind::Fence], "len={n}");
        }
    }

    #[test]
    fn two_tildes_is_continuation() {
        assert_eq!(kinds("~~\n"), vec![LineKind::Continuation]);
    }

    #[test]
    fn tildes_with_other_chars_is_continuation() {
        assert_eq!(kinds("~~~ x\n"), vec![LineKind::Continuation]);
        assert_eq!(kinds(" ~~~\n"), vec![LineKind::Continuation]);
    }

    #[test]
    fn comments_recognised() {
        assert_eq!(kinds("@! draft note\n"), vec![LineKind::Comment]);
    }

    #[test]
    fn at_alone_is_continuation() {
        assert_eq!(kinds("@\n"), vec![LineKind::Continuation]);
    }

    #[test]
    fn at_digit_is_continuation() {
        assert_eq!(kinds("@123\n"), vec![LineKind::Continuation]);
    }

    #[test]
    fn span_ranges_match_source() {
        let src = "@h1 a\n@p b\n";
        let lines = tokenize(src);
        assert_eq!(lines[0].slice(src), "@h1 a");
        assert_eq!(lines[1].slice(src), "@p b");
    }

    #[test]
    fn no_trailing_newline_still_emits_line() {
        let lines = tokenize("@p hi");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].kind, LineKind::BlockStart);
    }
}
