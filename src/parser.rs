//! Block-level parser for AGD. Consumes the line stream from `lexer`
//! and emits a `Document`. Single-pass, LL(1), no backtracking.

use std::collections::{BTreeMap, HashMap};

use crate::ast::{AttrValue, Block, BlockContent, BlockKind, Document, Inline, Span};
use crate::error::{AgdError, Result};
use crate::lexer::{tokenize, Line, LineKind};

pub fn parse(src: &str) -> Result<Document> {
    let lines = tokenize(src);
    let mut p = Parser {
        src,
        lines: &lines,
        pos: 0,
        blocks: Vec::new(),
        seen_ids: HashMap::new(),
    };
    while p.pos < p.lines.len() {
        let line = p.lines[p.pos];
        match line.kind {
            LineKind::BlockStart | LineKind::Comment => p.parse_block_at(line)?,
            LineKind::Empty => p.pos += 1,
            LineKind::ListItem => return Err(p.err(line, "stray list item `- ` outside @ul/@ol scope")),
            LineKind::QuoteLine => return Err(p.err(line, "stray quote line `> ` outside @quote scope")),
            LineKind::Continuation => {
                let snippet = line.slice(p.src);
                return Err(p.err(line, &format!("unexpected continuation: `{}`", trunc(snippet))));
            }
            LineKind::Fence => return Err(p.err(line, "stray fence `~~~` outside @code/@raw/@table block")),
        }
    }
    Ok(Document { blocks: p.blocks })
}

struct Parser<'a> {
    src: &'a str,
    lines: &'a [Line],
    pos: usize,
    blocks: Vec<Block>,
    /// id → first-seen line number (for duplicate diagnostics)
    seen_ids: HashMap<String, u32>,
}

impl<'a> Parser<'a> {
    fn err(&self, line: Line, msg: &str) -> AgdError {
        AgdError::Parse {
            line: line.line_no,
            col: 1,
            message: msg.to_string(),
        }
    }

    fn parse_block_at(&mut self, start: Line) -> Result<()> {
        let raw = start.slice(self.src);
        let span_start = start.span.start as usize;

        if start.kind == LineKind::Comment {
            // `@!<text>` — consume just this line as a comment block.
            let body = raw.strip_prefix("@!").unwrap_or("");
            let body = body.strip_prefix(' ').unwrap_or(body);
            let block = Block {
                kind: BlockKind::new(BlockKind::COMMENT),
                attrs: BTreeMap::new(),
                id: None,
                content: BlockContent::Inline(vec![Inline::Text(body.to_string())]),
                span: Span::new(span_start, start.span.end as usize),
            };
            self.blocks.push(block);
            self.pos += 1;
            return Ok(());
        }

        // Block-start line: `@<tag>[ args...][ [#id]]`
        let after_at = &raw[1..];
        let (tag, mut rest) = split_tag(after_at);
        validate_tag(tag, &start)?;
        rest = rest.strip_prefix(' ').unwrap_or(rest);

        // ID is always last on the line — strip from the end first.
        let (rest, id) = extract_trailing_id(rest, &start)?;
        if let Some(id_str) = id.as_deref() {
            self.register_id(id_str.to_string(), start.line_no)?;
        }

        let kind = BlockKind::new(tag);
        let mut block = match tag {
            BlockKind::META => Block {
                kind,
                attrs: parse_attrs(rest, &start)?,
                id,
                content: BlockContent::Empty,
                span: Span::new(span_start, start.span.end as usize),
            },
            BlockKind::P | BlockKind::H1 | BlockKind::H2 | BlockKind::H3 | BlockKind::H4 => {
                let inline = parse_inline(rest);
                Block {
                    kind,
                    attrs: BTreeMap::new(),
                    id,
                    content: BlockContent::Inline(inline),
                    span: Span::new(span_start, start.span.end as usize),
                }
            }
            BlockKind::REF => {
                let target = parse_ref_target(rest, &start)?;
                Block {
                    kind,
                    attrs: BTreeMap::new(),
                    id,
                    content: BlockContent::Inline(vec![Inline::Ref(target)]),
                    span: Span::new(span_start, start.span.end as usize),
                }
            }
            BlockKind::INCLUDE => Block {
                kind,
                attrs: parse_attrs(rest, &start)?,
                id,
                content: BlockContent::Empty,
                span: Span::new(span_start, start.span.end as usize),
            },
            _ => Block {
                kind,
                attrs: parse_attrs(rest, &start)?,
                id,
                content: BlockContent::Empty,
                span: Span::new(span_start, start.span.end as usize),
            },
        };

        self.pos += 1;

        // Multi-line bodies for ul/ol/quote/code/raw/table/x-*
        let tag_ref = tag.to_string();
        match tag_ref.as_str() {
            BlockKind::UL | BlockKind::OL => self.consume_list_items(&mut block)?,
            BlockKind::QUOTE => self.consume_quote_lines(&mut block)?,
            BlockKind::CODE | BlockKind::RAW | BlockKind::TABLE => self.consume_fence(&mut block, start)?,
            other if other.starts_with("x-") => {
                // Custom blocks: if next non-empty line is a fence, treat as raw.
                if self.peek_nonempty_is_fence() {
                    self.consume_fence(&mut block, start)?;
                }
            }
            _ => {}
        }

        self.blocks.push(block);
        Ok(())
    }

    fn peek_nonempty_is_fence(&self) -> bool {
        let mut i = self.pos;
        while i < self.lines.len() && self.lines[i].kind == LineKind::Empty {
            i += 1;
        }
        i < self.lines.len() && self.lines[i].kind == LineKind::Fence
    }

    fn consume_list_items(&mut self, block: &mut Block) -> Result<()> {
        let mut items = Vec::new();
        let mut end = block.span.end;
        while self.pos < self.lines.len() {
            let line = self.lines[self.pos];
            match line.kind {
                LineKind::ListItem => {
                    let text = line.slice(self.src);
                    let body = text.strip_prefix("- ").unwrap_or(text.strip_prefix('-').unwrap_or(text));
                    items.push(parse_inline(body));
                    end = line.span.end;
                    self.pos += 1;
                }
                LineKind::Empty | LineKind::BlockStart | LineKind::Comment => break,
                LineKind::QuoteLine => {
                    return Err(self.err(line, "quote line `> ` not allowed inside list — close list with blank line first"));
                }
                LineKind::Continuation => {
                    return Err(self.err(line, "list items must start with `- ` at column 0"));
                }
                LineKind::Fence => {
                    return Err(self.err(line, "fence `~~~` not allowed inside list"));
                }
            }
        }
        block.content = BlockContent::Items(items);
        block.span.end = end;
        Ok(())
    }

    fn consume_quote_lines(&mut self, block: &mut Block) -> Result<()> {
        let mut items = Vec::new();
        let mut end = block.span.end;
        while self.pos < self.lines.len() {
            let line = self.lines[self.pos];
            match line.kind {
                LineKind::QuoteLine => {
                    let text = line.slice(self.src);
                    let body = text.strip_prefix("> ").unwrap_or(text.strip_prefix('>').unwrap_or(text));
                    items.push(parse_inline(body));
                    end = line.span.end;
                    self.pos += 1;
                }
                LineKind::Empty | LineKind::BlockStart | LineKind::Comment => break,
                LineKind::ListItem => {
                    return Err(self.err(line, "list item `- ` not allowed inside quote"));
                }
                LineKind::Continuation => {
                    return Err(self.err(line, "quote lines must start with `> ` at column 0"));
                }
                LineKind::Fence => {
                    return Err(self.err(line, "fence `~~~` not allowed inside quote"));
                }
            }
        }
        block.content = BlockContent::Items(items);
        block.span.end = end;
        Ok(())
    }

    fn consume_fence(&mut self, block: &mut Block, opener: Line) -> Result<()> {
        // Skip leading empty lines between block-start and the opening fence.
        while self.pos < self.lines.len() && self.lines[self.pos].kind == LineKind::Empty {
            self.pos += 1;
        }
        if self.pos >= self.lines.len() || self.lines[self.pos].kind != LineKind::Fence {
            return Err(self.err(opener, "expected `~~~` fence after block start"));
        }
        let _open = self.lines[self.pos];
        self.pos += 1;
        let content_start = if self.pos < self.lines.len() {
            self.lines[self.pos].span.start as usize
        } else {
            self.src.len()
        };
        let mut close_end = None;
        while self.pos < self.lines.len() {
            let line = self.lines[self.pos];
            if line.kind == LineKind::Fence {
                close_end = Some(line.span.end as usize);
                self.pos += 1;
                break;
            }
            // Inside a fence, every other line kind is verbatim content.
            self.pos += 1;
        }
        let close_end = close_end.ok_or_else(|| AgdError::UnterminatedFence { line: opener.line_no })?;
        // Content = bytes from after-opening-fence newline to start of closing fence.
        // We need to walk back to find the start of the closing fence's line.
        // Simpler: rebuild by joining lines we passed through.
        // Cheaper: compute via spans.
        // The content spans `content_start` to `close_line_start` (start of `~~~` line).
        // We tracked close_end (end of `~~~` line). Backtrack to start of that line.
        let close_line_start = close_end - "~~~".len();
        let raw = if close_line_start >= content_start {
            // Strip trailing newline before the closing fence (the LF that ends the previous line).
            let mut s = &self.src[content_start..close_line_start];
            if let Some(stripped) = s.strip_suffix('\n') {
                s = stripped;
            }
            s.to_string()
        } else {
            String::new()
        };
        block.content = BlockContent::Fenced(raw);
        block.span.end = close_end as u32;
        Ok(())
    }

    fn register_id(&mut self, id: String, line_no: u32) -> Result<()> {
        if let Some(prev) = self.seen_ids.insert(id.clone(), line_no) {
            self.seen_ids.insert(id.clone(), prev); // keep first
            return Err(AgdError::DuplicateId {
                id,
                first_line: prev,
                dup_line: line_no,
            });
        }
        Ok(())
    }
}

// =====================================================================
//  Tag / id / attr / inline helpers
// =====================================================================

fn split_tag(after_at: &str) -> (&str, &str) {
    let mut end = 0;
    for (i, c) in after_at.char_indices() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    after_at.split_at(end)
}

fn validate_tag(tag: &str, line: &Line) -> Result<()> {
    if tag.is_empty() {
        return Err(AgdError::Parse {
            line: line.line_no,
            col: 2,
            message: "block tag is empty".into(),
        });
    }
    if BlockKind::BUILTINS.contains(&tag) || tag.starts_with("x-") {
        return Ok(());
    }
    Err(AgdError::InvalidTag {
        tag: tag.to_string(),
        line: line.line_no,
    })
}

fn extract_trailing_id<'a>(rest: &'a str, line: &Line) -> Result<(&'a str, Option<String>)> {
    let trimmed = rest.trim_end();
    if !trimmed.ends_with(']') {
        return Ok((rest, None));
    }
    let open = match trimmed.rfind("[#") {
        Some(i) => i,
        None => return Ok((rest, None)),
    };
    let inner = &trimmed[open + 2..trimmed.len() - 1];
    if inner.is_empty() {
        return Ok((rest, None));
    }
    // Optional `:hex` content-hash suffix is allowed but ignored at this layer.
    let (id_part, _hash) = match inner.find(':') {
        Some(i) => (&inner[..i], Some(&inner[i + 1..])),
        None => (inner, None),
    };
    if !is_valid_ident(id_part) {
        return Err(AgdError::InvalidId {
            id: id_part.to_string(),
            line: line.line_no,
        });
    }
    let head = trimmed[..open].trim_end();
    Ok((head, Some(id_part.to_string())))
}

fn parse_ref_target(rest: &str, line: &Line) -> Result<String> {
    let s = rest.trim();
    let target = s.strip_prefix('#').ok_or_else(|| AgdError::Parse {
        line: line.line_no,
        col: 6,
        message: "@ref must be followed by `#<id>`".into(),
    })?;
    if !is_valid_ident(target) {
        return Err(AgdError::InvalidId {
            id: target.to_string(),
            line: line.line_no,
        });
    }
    Ok(target.to_string())
}

fn parse_attrs(rest: &str, line: &Line) -> Result<BTreeMap<String, AttrValue>> {
    let mut out = BTreeMap::new();
    let s = rest.trim();
    if s.is_empty() {
        return Ok(out);
    }
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        // Read key
        let key_start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-') {
            i += 1;
        }
        let key = &s[key_start..i];
        if key.is_empty() {
            return Err(AgdError::InvalidAttr {
                line: line.line_no,
                message: format!("expected attribute key, got `{}`", trunc(&s[i..])),
            });
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            return Err(AgdError::InvalidAttr {
                line: line.line_no,
                message: format!("attribute `{key}` missing `=`"),
            });
        }
        i += 1; // skip '='
        // Read value
        let value = if i < bytes.len() && bytes[i] == b'"' {
            i += 1;
            let mut v = String::new();
            let mut closed = false;
            while i < bytes.len() {
                let b = bytes[i];
                if b == b'\\' && i + 1 < bytes.len() {
                    // Escape sequence: copy next byte verbatim.
                    v.push(bytes[i + 1] as char);
                    i += 2;
                    continue;
                }
                if b == b'"' {
                    i += 1;
                    closed = true;
                    break;
                }
                // Push next UTF-8 char
                let ch_len = utf8_char_len(b);
                v.push_str(&s[i..i + ch_len]);
                i += ch_len;
            }
            if !closed {
                return Err(AgdError::InvalidAttr {
                    line: line.line_no,
                    message: format!("attribute `{key}` has unterminated quoted value"),
                });
            }
            AttrValue::Str(v)
        } else {
            let v_start = i;
            while i < bytes.len() && bytes[i] != b' ' {
                i += 1;
            }
            let v = &s[v_start..i];
            classify_attr_value(v)
        };
        out.insert(key.to_string(), value);
    }
    Ok(out)
}

fn classify_attr_value(v: &str) -> AttrValue {
    match v {
        "true" => AttrValue::Bool(true),
        "false" => AttrValue::Bool(false),
        _ => {
            if let Ok(n) = v.parse::<i64>() {
                AttrValue::Int(n)
            } else {
                AttrValue::Str(v.to_string())
            }
        }
    }
}

pub(crate) fn is_valid_ident(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let bytes = s.as_bytes();
    if !(bytes[0].is_ascii_alphabetic() || bytes[0] == b'_') {
        return false;
    }
    bytes[1..].iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
}

// =====================================================================
//  Inline parser
// =====================================================================

pub fn parse_inline(s: &str) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        // Inline references are not part of v0.1 inline grammar — they
        // exist only as block-level `@ref #id`. An `@` inside running
        // inline text is treated as a literal character.
        match b {
            b'*' | b'_' | b'`' => {
                // Find matching closer
                let close = find_close(bytes, i + 1, b);
                if let Some(end) = close {
                    flush(&mut out, &mut buf);
                    let content = s[i + 1..end].to_string();
                    let node = match b {
                        b'*' => Inline::Bold(content),
                        b'_' => Inline::Italic(content),
                        b'`' => Inline::Code(content),
                        _ => unreachable!(),
                    };
                    out.push(node);
                    i = end + 1;
                    continue;
                } else {
                    // No closer → degrade to plain text
                    buf.push(b as char);
                    i += 1;
                }
            }
            _ => {
                // Push the next UTF-8 char (handles multibyte safely)
                let ch_len = utf8_char_len(b);
                buf.push_str(&s[i..i + ch_len]);
                i += ch_len;
            }
        }
    }
    flush(&mut out, &mut buf);
    out
}

fn flush(out: &mut Vec<Inline>, buf: &mut String) {
    if !buf.is_empty() {
        out.push(Inline::Text(std::mem::take(buf)));
    }
}

fn find_close(bytes: &[u8], from: usize, delim: u8) -> Option<usize> {
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == delim {
            return Some(i);
        }
        i += 1;
    }
    None
}

pub(crate) fn utf8_char_len(b: u8) -> usize {
    if b < 0x80 { 1 }
    else if b < 0xC0 { 1 }       // continuation byte — shouldn't happen at start, treat as 1
    else if b < 0xE0 { 2 }
    else if b < 0xF0 { 3 }
    else { 4 }
}

// =====================================================================
//  Misc
// =====================================================================

fn trunc(s: &str) -> String {
    if s.len() > 32 {
        format!("{}…", &s[..32.min(s.len())])
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_empty_document() {
        let doc = parse("").unwrap();
        assert!(doc.blocks.is_empty());
    }

    #[test]
    fn parses_single_heading() {
        let doc = parse("@h1 Hello\n").unwrap();
        assert_eq!(doc.blocks.len(), 1);
        assert_eq!(doc.blocks[0].kind.as_str(), "h1");
        assert_eq!(doc.blocks[0].content.as_inline().unwrap(), &[Inline::Text("Hello".into())]);
    }

    #[test]
    fn parses_heading_with_id() {
        let doc = parse("@h1 Hello [#intro]\n").unwrap();
        assert_eq!(doc.blocks[0].id.as_deref(), Some("intro"));
    }

    #[test]
    fn parses_meta_attrs() {
        let doc = parse("@meta title=\"My Doc\" v=1\n").unwrap();
        let m = &doc.blocks[0];
        assert_eq!(m.kind.as_str(), "meta");
        assert_eq!(m.attrs.get("title"), Some(&AttrValue::Str("My Doc".into())));
        assert_eq!(m.attrs.get("v"), Some(&AttrValue::Int(1)));
    }

    #[test]
    fn parses_list() {
        let doc = parse("@ul\n- one\n- two\n- three\n").unwrap();
        let items = doc.blocks[0].content.as_items().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0], vec![Inline::Text("one".into())]);
    }

    #[test]
    fn parses_code_fence() {
        let src = "@code lang=rust\n~~~\nfn main() {}\n~~~\n";
        let doc = parse(src).unwrap();
        assert_eq!(doc.blocks[0].content.as_fenced().unwrap(), "fn main() {}");
        assert_eq!(doc.blocks[0].attrs.get("lang"), Some(&AttrValue::Str("rust".into())));
    }

    #[test]
    fn duplicate_ids_rejected() {
        let src = "@h1 A [#x]\n@h2 B [#x]\n";
        assert!(matches!(parse(src), Err(AgdError::DuplicateId { .. })));
    }

    #[test]
    fn unknown_tag_rejected() {
        assert!(matches!(parse("@unknown\n"), Err(AgdError::InvalidTag { .. })));
    }

    #[test]
    fn custom_x_tag_accepted() {
        let doc = parse("@x-diagram type=flow\n").unwrap();
        assert_eq!(doc.blocks[0].kind.as_str(), "x-diagram");
    }

    #[test]
    fn inline_emphasis_parsed() {
        let inl = parse_inline("hello *world* and _italic_ and `code`");
        assert_eq!(
            inl,
            vec![
                Inline::Text("hello ".into()),
                Inline::Bold("world".into()),
                Inline::Text(" and ".into()),
                Inline::Italic("italic".into()),
                Inline::Text(" and ".into()),
                Inline::Code("code".into()),
            ]
        );
    }

    #[test]
    fn inline_at_is_plain_text() {
        // v0.1: `@` inside running text is literal — no inline references.
        let inl = parse_inline("see @ref #intro for context");
        assert_eq!(inl, vec![Inline::Text("see @ref #intro for context".into())]);
    }

    #[test]
    fn unmatched_emphasis_degrades_to_text() {
        let inl = parse_inline("a *b c");
        assert_eq!(inl, vec![Inline::Text("a *b c".into())]);
    }

    #[test]
    fn ref_block_parsed() {
        let doc = parse("@ref #intro\n").unwrap();
        assert!(matches!(&doc.blocks[0].content,
            BlockContent::Inline(v) if v.len() == 1 && matches!(&v[0], Inline::Ref(s) if s == "intro")));
    }

    #[test]
    fn quote_block_parsed() {
        let doc = parse("@quote source=\"x\"\n> hello\n> world\n").unwrap();
        let items = doc.blocks[0].content.as_items().unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn unterminated_fence_errors() {
        assert!(matches!(parse("@code\n~~~\nstuff\n"), Err(AgdError::UnterminatedFence { .. })));
    }

    #[test]
    fn comment_preserved() {
        let doc = parse("@! this is a note\n").unwrap();
        assert_eq!(doc.blocks[0].kind.as_str(), "!");
    }
}
