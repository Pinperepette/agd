//! AGD → CommonMark.

use crate::ast::{Block, BlockContent, Inline};
use crate::serializer::render_inline;
use crate::Document;

pub fn to_markdown(doc: &Document) -> String {
    let mut out = String::new();
    for (idx, block) in doc.blocks.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        write_block(&mut out, block);
    }
    out
}

fn write_block(out: &mut String, block: &Block) {
    match block.kind.as_str() {
        "meta" => {
            out.push_str("<!-- ");
            for (k, v) in &block.attrs {
                out.push_str(k);
                out.push('=');
                match v {
                    crate::ast::AttrValue::Str(s) => out.push_str(s),
                    crate::ast::AttrValue::Int(n) => out.push_str(&n.to_string()),
                    crate::ast::AttrValue::Bool(b) => out.push_str(&b.to_string()),
                }
                out.push(' ');
            }
            out.push_str("-->\n");
        }
        "h1" | "h2" | "h3" | "h4" => {
            let n = match block.kind.as_str() {
                "h1" => 1,
                "h2" => 2,
                "h3" => 3,
                _ => 4,
            };
            out.push_str(&"#".repeat(n));
            out.push(' ');
            if let BlockContent::Inline(v) = &block.content {
                out.push_str(&inline_md(v));
            }
            out.push('\n');
        }
        "p" => {
            if let BlockContent::Inline(v) = &block.content {
                out.push_str(&inline_md(v));
                out.push('\n');
            }
        }
        "ul" => {
            if let BlockContent::Items(items) = &block.content {
                for it in items {
                    out.push_str("- ");
                    out.push_str(&inline_md(it));
                    out.push('\n');
                }
            }
        }
        "ol" => {
            if let BlockContent::Items(items) = &block.content {
                for (i, it) in items.iter().enumerate() {
                    out.push_str(&format!("{}. ", i + 1));
                    out.push_str(&inline_md(it));
                    out.push('\n');
                }
            }
        }
        "code" | "raw" | "table" => {
            let lang = block.attrs.get("lang").and_then(|v| v.as_str()).unwrap_or("");
            out.push_str("```");
            out.push_str(lang);
            out.push('\n');
            if let BlockContent::Fenced(s) = &block.content {
                out.push_str(s);
                if !s.ends_with('\n') && !s.is_empty() {
                    out.push('\n');
                }
            }
            out.push_str("```\n");
        }
        "quote" => {
            if let BlockContent::Items(items) = &block.content {
                for it in items {
                    out.push_str("> ");
                    out.push_str(&inline_md(it));
                    out.push('\n');
                }
            }
        }
        "ref" => {
            if let BlockContent::Inline(v) = &block.content {
                if let Some(Inline::Ref(target)) = v.first() {
                    out.push_str(&format!("[#{target}](#{target})\n"));
                }
            }
        }
        "!" => {
            if let BlockContent::Inline(v) = &block.content {
                out.push_str("<!-- ");
                out.push_str(&render_inline(v));
                out.push_str(" -->\n");
            }
        }
        _ => {
            // x-* and unknown → emit as fenced raw
            out.push_str("```");
            out.push_str(block.kind.as_str());
            out.push('\n');
            if let BlockContent::Fenced(s) = &block.content {
                out.push_str(s);
                if !s.ends_with('\n') {
                    out.push('\n');
                }
            }
            out.push_str("```\n");
        }
    }
}

fn inline_md(nodes: &[Inline]) -> String {
    let mut s = String::new();
    for n in nodes {
        match n {
            Inline::Text(t) => s.push_str(t),
            Inline::Bold(t) => {
                s.push_str("**");
                s.push_str(t);
                s.push_str("**");
            }
            Inline::Italic(t) => {
                s.push('*');
                s.push_str(t);
                s.push('*');
            }
            Inline::Code(t) => {
                s.push('`');
                s.push_str(t);
                s.push('`');
            }
            Inline::Ref(target) => {
                s.push('[');
                s.push('#');
                s.push_str(target);
                s.push_str("](#");
                s.push_str(target);
                s.push(')');
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    #[test]
    fn heading_to_md() {
        let doc = parse("@h2 Hello\n").unwrap();
        assert_eq!(to_markdown(&doc), "## Hello\n");
    }

    #[test]
    fn list_to_md() {
        let doc = parse("@ul\n- a\n- b\n").unwrap();
        assert!(to_markdown(&doc).contains("- a"));
    }

    #[test]
    fn code_to_md() {
        let doc = parse("@code lang=rust\n~~~\nfn main(){}\n~~~\n").unwrap();
        let md = to_markdown(&doc);
        assert!(md.contains("```rust"));
        assert!(md.contains("fn main(){}"));
    }
}
