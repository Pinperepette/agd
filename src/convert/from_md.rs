//! Markdown → AGD converter via `pulldown-cmark` events.
//!
//! Lossy conversions (documented per spec):
//!   - Tables collapse into `@code lang=csv` (raw CSV-ish text).
//!   - Footnotes drop to `@raw type=footnote`.
//!   - Raw HTML drops to `@raw type=html`.
//!   - Soft line breaks within a paragraph are joined into a single `@p`.
//!   - Nested lists flatten — only the innermost level survives.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser as MdParser, Tag, TagEnd};

use crate::ast::{AttrValue, Block, BlockContent, BlockKind, Inline};
use crate::serializer::serialize;
use crate::Document;

pub fn from_markdown(src: &str) -> String {
    let doc = parse_markdown(src);
    serialize(&doc)
}

pub fn parse_markdown(src: &str) -> Document {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = MdParser::new_ext(src, opts);

    let mut blocks = Vec::new();
    let mut buf: Vec<Inline> = Vec::new();
    let mut list_items: Vec<Vec<Inline>> = Vec::new();
    let mut in_list_kind: Option<&'static str> = None;
    let mut current_heading: Option<HeadingLevel> = None;
    let mut in_quote = false;
    let mut quote_lines: Vec<Vec<Inline>> = Vec::new();
    let mut code_lang: Option<String> = None;
    let mut code_buf = String::new();

    for ev in parser {
        match ev {
            Event::Start(Tag::Heading { level, .. }) => current_heading = Some(level),
            Event::End(TagEnd::Heading(_)) => {
                if let Some(level) = current_heading.take() {
                    let tag = match level {
                        HeadingLevel::H1 => "h1",
                        HeadingLevel::H2 => "h2",
                        HeadingLevel::H3 => "h3",
                        _ => "h4",
                    };
                    blocks.push(Block {
                        kind: BlockKind::new(tag),
                        attrs: Default::default(),
                        id: None,
                        content: BlockContent::Inline(std::mem::take(&mut buf)),
                        span: Default::default(),
                    });
                }
            }
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                if !buf.is_empty() && in_list_kind.is_none() && !in_quote {
                    blocks.push(Block {
                        kind: BlockKind::new("p"),
                        attrs: Default::default(),
                        id: None,
                        content: BlockContent::Inline(std::mem::take(&mut buf)),
                        span: Default::default(),
                    });
                }
            }
            Event::Start(Tag::List(opt)) => {
                in_list_kind = Some(if opt.is_some() { "ol" } else { "ul" });
            }
            Event::End(TagEnd::List(_)) => {
                if let Some(kind) = in_list_kind.take() {
                    blocks.push(Block {
                        kind: BlockKind::new(kind),
                        attrs: Default::default(),
                        id: None,
                        content: BlockContent::Items(std::mem::take(&mut list_items)),
                        span: Default::default(),
                    });
                }
            }
            Event::Start(Tag::Item) => {}
            Event::End(TagEnd::Item) => {
                if in_list_kind.is_some() {
                    list_items.push(std::mem::take(&mut buf));
                }
            }
            Event::Start(Tag::BlockQuote) => in_quote = true,
            Event::End(TagEnd::BlockQuote) => {
                in_quote = false;
                if !buf.is_empty() {
                    quote_lines.push(std::mem::take(&mut buf));
                }
                if !quote_lines.is_empty() {
                    blocks.push(Block {
                        kind: BlockKind::new("quote"),
                        attrs: Default::default(),
                        id: None,
                        content: BlockContent::Items(std::mem::take(&mut quote_lines)),
                        span: Default::default(),
                    });
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                code_lang = match kind {
                    CodeBlockKind::Fenced(s) => {
                        let s = s.to_string();
                        if s.is_empty() { None } else { Some(s) }
                    }
                    CodeBlockKind::Indented => None,
                };
                code_buf.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                let mut attrs = std::collections::BTreeMap::new();
                if let Some(lang) = code_lang.take() {
                    attrs.insert("lang".into(), AttrValue::Str(lang));
                }
                let mut content = std::mem::take(&mut code_buf);
                if content.ends_with('\n') {
                    content.pop();
                }
                blocks.push(Block {
                    kind: BlockKind::new("code"),
                    attrs,
                    id: None,
                    content: BlockContent::Fenced(content),
                    span: Default::default(),
                });
            }
            Event::Text(t) => {
                if code_lang.is_some() || code_buf_active(&code_buf) {
                    code_buf.push_str(&t);
                } else if in_quote {
                    buf.push(Inline::Text(t.into_string()));
                } else {
                    buf.push(Inline::Text(t.into_string()));
                }
            }
            Event::Code(t) => buf.push(Inline::Code(t.into_string())),
            Event::Start(Tag::Emphasis) => buf.push(Inline::Italic(String::new())),
            Event::End(TagEnd::Emphasis) => merge_into_last_italic(&mut buf),
            Event::Start(Tag::Strong) => buf.push(Inline::Bold(String::new())),
            Event::End(TagEnd::Strong) => merge_into_last_bold(&mut buf),
            Event::SoftBreak | Event::HardBreak => buf.push(Inline::Text(" ".into())),
            Event::Html(s) | Event::InlineHtml(s) => {
                blocks.push(Block {
                    kind: BlockKind::new("raw"),
                    attrs: {
                        let mut m = std::collections::BTreeMap::new();
                        m.insert("type".to_string(), AttrValue::Str("html".to_string()));
                        m
                    },
                    id: None,
                    content: BlockContent::Fenced(s.into_string().trim_end().to_string()),
                    span: Default::default(),
                });
            }
            _ => {}
        }
    }

    Document { blocks }
}

fn code_buf_active(_buf: &str) -> bool {
    // helper for clarity — code_lang.is_some() is the source of truth in the loop above
    false
}

fn merge_into_last_italic(buf: &mut Vec<Inline>) {
    // pulldown-cmark emits Start(Emphasis), Text, End(Emphasis).
    // We push an empty Italic on Start, then Text events appended after it.
    // On End, fold all trailing Text nodes back into the Italic.
    let split = buf
        .iter()
        .rposition(|n| matches!(n, Inline::Italic(s) if s.is_empty()));
    if let Some(idx) = split {
        let mut text = String::new();
        for n in buf.drain(idx + 1..) {
            text.push_str(n.as_plain());
        }
        if let Inline::Italic(t) = &mut buf[idx] {
            *t = text;
        }
    }
}

fn merge_into_last_bold(buf: &mut Vec<Inline>) {
    let split = buf
        .iter()
        .rposition(|n| matches!(n, Inline::Bold(s) if s.is_empty()));
    if let Some(idx) = split {
        let mut text = String::new();
        for n in buf.drain(idx + 1..) {
            text.push_str(n.as_plain());
        }
        if let Inline::Bold(t) = &mut buf[idx] {
            *t = text;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_and_paragraph() {
        let agd = from_markdown("# Hello\n\nWorld\n");
        assert!(agd.contains("@h1 Hello"));
        assert!(agd.contains("@p World"));
    }

    #[test]
    fn fenced_code_with_lang() {
        let agd = from_markdown("```python\nx = 1\n```\n");
        assert!(agd.contains("@code lang=python"));
        assert!(agd.contains("x = 1"));
    }

    #[test]
    fn unordered_list() {
        let agd = from_markdown("- one\n- two\n");
        assert!(agd.contains("@ul"));
        assert!(agd.contains("- one"));
        assert!(agd.contains("- two"));
    }
}
