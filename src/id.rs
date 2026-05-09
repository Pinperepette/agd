//! Block ID management: validation, auto-assignment, content hashing.

use sha1::{Digest, Sha1};

use crate::ast::{Block, BlockContent, BlockKind, Document, Inline};
use crate::serializer::render_inline;

/// Auto-assign IDs to every anchorable block that doesn't already have one.
/// Anchorable = headings, code, raw, table, quote, ul, ol, custom (x-*).
/// Strategy:
///   - Headings: slug of the text content.
///   - Other blocks: `<tag>-<n>` where n is the running index of that tag.
/// Existing IDs are preserved. Generated IDs are deduplicated by suffix.
pub fn auto_assign(doc: &mut Document) {
    let mut taken: std::collections::HashSet<String> =
        doc.blocks.iter().filter_map(|b| b.id.clone()).collect();
    let mut tag_counter: std::collections::HashMap<String, usize> = Default::default();

    for block in &mut doc.blocks {
        if block.id.is_some() || !is_anchorable(&block.kind) {
            continue;
        }
        let candidate = if block.kind.is_heading() {
            slug_for_heading(block).unwrap_or_else(|| format!("{}-1", block.kind))
        } else {
            let n = tag_counter.entry(block.kind.to_string()).and_modify(|v| *v += 1).or_insert(1);
            format!("{}-{}", block.kind, n)
        };
        let id = dedup(candidate, &taken);
        taken.insert(id.clone());
        block.id = Some(id);
    }
}

/// Strip every block ID. Useful for normalising a doc before diffing.
pub fn strip_all(doc: &mut Document) {
    for block in &mut doc.blocks {
        block.id = None;
    }
}

/// 8-character SHA-1 hex prefix of the canonical block bytes — used for
/// content-hash verification when an agent edits a block.
pub fn content_hash(block: &Block) -> String {
    let mut hasher = Sha1::new();
    let canonical = canonical_for_hash(block);
    hasher.update(canonical.as_bytes());
    let digest = hasher.finalize();
    hex8(&digest[..])
}

fn canonical_for_hash(block: &Block) -> String {
    // Hash includes kind + attrs (sorted) + content, but NOT the id itself
    // (so renaming doesn't invalidate the hash).
    let mut s = String::new();
    s.push_str(block.kind.as_str());
    for (k, v) in &block.attrs {
        s.push('|');
        s.push_str(k);
        s.push('=');
        match v {
            crate::ast::AttrValue::Bool(b) => s.push_str(&b.to_string()),
            crate::ast::AttrValue::Int(n) => s.push_str(&n.to_string()),
            crate::ast::AttrValue::Str(t) => s.push_str(t),
        }
    }
    s.push('|');
    match &block.content {
        BlockContent::Inline(v) => s.push_str(&render_inline(v)),
        BlockContent::Items(items) => {
            for item in items {
                s.push_str(&render_inline(item));
                s.push('\n');
            }
        }
        BlockContent::Fenced(t) => s.push_str(t),
        BlockContent::Empty => {}
    }
    s
}

fn hex8(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(8);
    for b in bytes.iter().take(4) {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

fn is_anchorable(kind: &BlockKind) -> bool {
    matches!(
        kind.as_str(),
        "h1" | "h2" | "h3" | "h4" | "code" | "raw" | "table" | "quote" | "ul" | "ol"
    ) || kind.is_custom()
}

fn slug_for_heading(block: &Block) -> Option<String> {
    let inlines = block.content.as_inline()?;
    let mut text = String::new();
    for n in inlines {
        match n {
            Inline::Text(t) | Inline::Bold(t) | Inline::Italic(t) | Inline::Code(t) => text.push_str(t),
            Inline::Ref(_) => {}
        }
    }
    Some(slugify(&text))
}

fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_dash = false;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !out.is_empty() && !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_end_matches('-').to_string();
    if trimmed.is_empty() || !trimmed.as_bytes()[0].is_ascii_alphabetic() {
        format!("h-{}", trimmed)
    } else {
        trimmed
    }
}

fn dedup(base: String, taken: &std::collections::HashSet<String>) -> String {
    if !taken.contains(&base) {
        return base;
    }
    for n in 2.. {
        let candidate = format!("{base}-{n}");
        if !taken.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    #[test]
    fn auto_assign_headings_get_slugs() {
        let mut doc = parse("@h1 Hello World\n@h2 Sub-Section\n").unwrap();
        auto_assign(&mut doc);
        assert_eq!(doc.blocks[0].id.as_deref(), Some("hello-world"));
        assert_eq!(doc.blocks[1].id.as_deref(), Some("sub-section"));
    }

    #[test]
    fn auto_assign_code_blocks() {
        let mut doc = parse("@code\n~~~\na\n~~~\n@code\n~~~\nb\n~~~\n").unwrap();
        auto_assign(&mut doc);
        assert_eq!(doc.blocks[0].id.as_deref(), Some("code-1"));
        assert_eq!(doc.blocks[1].id.as_deref(), Some("code-2"));
    }

    #[test]
    fn duplicate_slugs_get_suffix() {
        let mut doc = parse("@h1 Foo\n@h2 Foo\n").unwrap();
        auto_assign(&mut doc);
        assert_eq!(doc.blocks[0].id.as_deref(), Some("foo"));
        assert_eq!(doc.blocks[1].id.as_deref(), Some("foo-2"));
    }

    #[test]
    fn strip_clears_all() {
        let mut doc = parse("@h1 X [#a]\n@p Y [#b]\n").unwrap();
        strip_all(&mut doc);
        assert!(doc.blocks.iter().all(|b| b.id.is_none()));
    }

    #[test]
    fn content_hash_is_eight_hex() {
        let doc = parse("@p hello\n").unwrap();
        let h = content_hash(&doc.blocks[0]);
        assert_eq!(h.len(), 8);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
