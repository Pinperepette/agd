//! Canonical AGD serializer. Produces byte-stable output:
//!   - LF line endings
//!   - single space between tokens
//!   - attributes sorted alphabetically (BTreeMap iteration order)
//!   - ID, when present, always last on the block-start line
//!   - no trailing whitespace
//!   - bare keywords (true/false/ints) unquoted; strings quoted only when needed
//!   - inline emphasis emitted in the original delimiter form

use std::fmt::Write;

use crate::ast::{AttrValue, Block, BlockContent, Document, Inline};

pub fn serialize(doc: &Document) -> String {
    let mut out = String::with_capacity(256);
    for (idx, block) in doc.blocks.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        write_block(&mut out, block);
    }
    out
}

fn write_block(out: &mut String, block: &Block) {
    let tag = block.kind.as_str();

    // Comments are special: `@! <text>` on a single line.
    if tag == "!" {
        out.push_str("@!");
        if let BlockContent::Inline(v) = &block.content {
            let body = render_inline(v);
            if !body.is_empty() {
                out.push(' ');
                out.push_str(&body);
            }
        }
        out.push('\n');
        return;
    }

    out.push('@');
    out.push_str(tag);

    // Attributes (sorted by key in BTreeMap).
    for (k, v) in &block.attrs {
        out.push(' ');
        out.push_str(k);
        out.push('=');
        write_attr_value(out, v);
    }

    // Inline body for inline-bearing tags. `@ref` is special: render only
    // the bare `#target` form to avoid emitting the tag twice.
    if let BlockContent::Inline(v) = &block.content {
        if tag == "ref" {
            if let Some(Inline::Ref(target)) = v.first() {
                out.push(' ');
                out.push('#');
                out.push_str(target);
            }
        } else {
            let body = render_inline(v);
            if !body.is_empty() {
                out.push(' ');
                out.push_str(&body);
            }
        }
    }

    // ID always last.
    if let Some(id) = &block.id {
        out.push(' ');
        out.push_str("[#");
        out.push_str(id);
        out.push(']');
    }
    out.push('\n');

    // Multi-line bodies.
    match &block.content {
        BlockContent::Items(items) => {
            let prefix = if tag == "quote" { "> " } else { "- " };
            for item in items {
                out.push_str(prefix);
                out.push_str(&render_inline(item));
                out.push('\n');
            }
        }
        BlockContent::Fenced(s) => {
            // Variable-length fence: pick the smallest length ≥ 3 that is
            // strictly longer than any tilde-only line inside the body, so
            // bodies containing `~~~` round-trip losslessly.
            let fence_len = pick_fence_len(s);
            let fence: String = "~".repeat(fence_len);
            out.push_str(&fence);
            out.push('\n');
            if !s.is_empty() {
                out.push_str(s);
                out.push('\n');
            }
            out.push_str(&fence);
            out.push('\n');
        }
        _ => {}
    }
}

fn pick_fence_len(body: &str) -> usize {
    let mut max_run = 0usize;
    for line in body.split('\n') {
        if !line.is_empty() && line.bytes().all(|b| b == b'~') {
            max_run = max_run.max(line.len());
        }
    }
    std::cmp::max(3, max_run + 1)
}

fn write_attr_value(out: &mut String, v: &AttrValue) {
    match v {
        AttrValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        AttrValue::Int(n) => write!(out, "{n}").unwrap(),
        AttrValue::Str(s) => {
            if needs_quoting(s) {
                out.push('"');
                for ch in s.chars() {
                    if ch == '"' || ch == '\\' {
                        out.push('\\');
                    }
                    out.push(ch);
                }
                out.push('"');
            } else {
                out.push_str(s);
            }
        }
    }
}

fn needs_quoting(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    // A bareword must be a valid identifier-like token, distinct from int/bool.
    if s == "true" || s == "false" {
        return true;
    }
    if s.parse::<i64>().is_ok() {
        return true;
    }
    s.chars().any(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
}

pub fn render_inline(nodes: &[Inline]) -> String {
    let mut s = String::new();
    for n in nodes {
        match n {
            Inline::Text(t) => s.push_str(t),
            Inline::Bold(t) => {
                s.push('*');
                s.push_str(t);
                s.push('*');
            }
            Inline::Italic(t) => {
                s.push('_');
                s.push_str(t);
                s.push('_');
            }
            Inline::Code(t) => {
                s.push('`');
                s.push_str(t);
                s.push('`');
            }
            Inline::Ref(id) => {
                s.push_str("@ref #");
                s.push_str(id);
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn round(src: &str) -> String {
        let doc = parse(src).unwrap();
        serialize(&doc)
    }

    #[test]
    fn roundtrip_simple_heading() {
        let src = "@h1 Hello [#intro]\n";
        assert_eq!(round(src), src);
    }

    #[test]
    fn roundtrip_meta_sorted() {
        // Input has b before a; canonical output sorts alphabetically.
        let src = "@meta b=2 a=1\n";
        assert_eq!(round(src), "@meta a=1 b=2\n");
    }

    #[test]
    fn roundtrip_list() {
        let src = "@ul [#l1]\n- one\n- two\n";
        assert_eq!(round(src), src);
    }

    #[test]
    fn roundtrip_code_fence() {
        let src = "@code lang=rust\n~~~\nfn main() {}\n~~~\n";
        assert_eq!(round(src), src);
    }

    #[test]
    fn roundtrip_inline_emphasis() {
        let src = "@p hello *world* and _it_ and `code`\n";
        assert_eq!(round(src), src);
    }

    #[test]
    fn roundtrip_inline_ref() {
        let src = "@p see @ref #x for context\n";
        assert_eq!(round(src), src);
    }

    #[test]
    fn quoted_value_preserved() {
        let src = "@meta title=\"hello world\"\n";
        assert_eq!(round(src), src);
    }

    #[test]
    fn comment_preserved() {
        let src = "@! draft\n";
        assert_eq!(round(src), src);
    }

    #[test]
    fn quote_block_roundtrip() {
        let src = "@quote source=rfc\n> first\n> second\n";
        assert_eq!(round(src), src);
    }

    #[test]
    fn body_with_internal_three_tildes_roundtrips() {
        // Critical: a body containing a `~~~` line must round-trip without
        // corruption. Source uses a 4-tilde wrapper; serializer must keep
        // it ≥4 because the body still contains `~~~`.
        let src = "@x-note\n~~~~\nbefore\n~~~\nafter\n~~~~\n";
        let doc = parse(src).unwrap();
        let body = doc.blocks[0].content.as_fenced().unwrap();
        assert_eq!(body, "before\n~~~\nafter");

        let out = serialize(&doc);
        assert!(out.contains("~~~~\n"), "expected ≥4-tilde fence in output: {out}");

        // Re-parse to confirm round-trip stability.
        let doc2 = parse(&out).unwrap();
        assert_eq!(doc2.blocks[0].content.as_fenced().unwrap(), body);
    }

    #[test]
    fn pick_fence_len_picks_minimum_safe_length() {
        assert_eq!(pick_fence_len(""), 3);
        assert_eq!(pick_fence_len("plain content\n"), 3);
        assert_eq!(pick_fence_len("a\n~~\nb"), 3);          // ~~ is < 3, ignored
        assert_eq!(pick_fence_len("a\n~~~\nb"), 4);         // beat ~~~
        assert_eq!(pick_fence_len("a\n~~~~\nb\n~~~\n"), 5); // beat the longest
    }
}
