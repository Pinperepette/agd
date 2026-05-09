//! AGD → minimal semantic HTML. Used to render the spec as a webpage and
//! to support the token benchmark's HTML-equivalent baseline.

use crate::ast::{Block, BlockContent, Inline};
use crate::Document;

pub fn to_html(doc: &Document) -> String {
    let mut out = String::with_capacity(doc.blocks.len() * 64);
    for block in &doc.blocks {
        write_block(&mut out, block);
    }
    out
}

fn write_block(out: &mut String, block: &Block) {
    let id_attr = match &block.id {
        Some(id) => format!(" id=\"{}\"", escape_attr(id)),
        None => String::new(),
    };
    match block.kind.as_str() {
        "meta" => { /* skipped — would map to <head> tags in a full doc */ }
        "h1" | "h2" | "h3" | "h4" => {
            let tag = block.kind.as_str();
            out.push_str(&format!("<{tag}{id_attr}>"));
            if let BlockContent::Inline(v) = &block.content {
                out.push_str(&inline_html(v));
            }
            out.push_str(&format!("</{tag}>\n"));
        }
        "p" => {
            out.push_str(&format!("<p{id_attr}>"));
            if let BlockContent::Inline(v) = &block.content {
                out.push_str(&inline_html(v));
            }
            out.push_str("</p>\n");
        }
        "ul" | "ol" => {
            let tag = block.kind.as_str();
            out.push_str(&format!("<{tag}{id_attr}>\n"));
            if let BlockContent::Items(items) = &block.content {
                for it in items {
                    out.push_str("  <li>");
                    out.push_str(&inline_html(it));
                    out.push_str("</li>\n");
                }
            }
            out.push_str(&format!("</{tag}>\n"));
        }
        "code" => {
            let lang = block.attrs.get("lang").and_then(|v| v.as_str()).unwrap_or("");
            let cls = if lang.is_empty() { String::new() } else { format!(" class=\"language-{}\"", escape_attr(lang)) };
            out.push_str(&format!("<pre{id_attr}><code{cls}>"));
            if let BlockContent::Fenced(s) = &block.content {
                out.push_str(&escape_html(s));
            }
            out.push_str("</code></pre>\n");
        }
        "raw" | "table" => {
            out.push_str(&format!("<pre{id_attr}>"));
            if let BlockContent::Fenced(s) = &block.content {
                out.push_str(&escape_html(s));
            }
            out.push_str("</pre>\n");
        }
        "quote" => {
            out.push_str(&format!("<blockquote{id_attr}>\n"));
            if let BlockContent::Items(items) = &block.content {
                for it in items {
                    out.push_str("  <p>");
                    out.push_str(&inline_html(it));
                    out.push_str("</p>\n");
                }
            }
            out.push_str("</blockquote>\n");
        }
        "ref" => {
            if let BlockContent::Inline(v) = &block.content {
                if let Some(Inline::Ref(t)) = v.first() {
                    out.push_str(&format!(
                        "<p><a href=\"#{t}\">#{t}</a></p>\n",
                        t = escape_attr(t)
                    ));
                }
            }
        }
        "!" => {
            if let BlockContent::Inline(v) = &block.content {
                out.push_str("<!-- ");
                for n in v {
                    out.push_str(n.as_plain());
                }
                out.push_str(" -->\n");
            }
        }
        _ => {
            out.push_str(&format!("<div data-agd-tag=\"{}\"{id_attr}>", escape_attr(block.kind.as_str())));
            if let BlockContent::Fenced(s) = &block.content {
                out.push_str(&escape_html(s));
            }
            out.push_str("</div>\n");
        }
    }
}

fn inline_html(nodes: &[Inline]) -> String {
    let mut s = String::new();
    for n in nodes {
        match n {
            Inline::Text(t) => s.push_str(&escape_html(t)),
            Inline::Bold(t) => {
                s.push_str("<strong>");
                s.push_str(&escape_html(t));
                s.push_str("</strong>");
            }
            Inline::Italic(t) => {
                s.push_str("<em>");
                s.push_str(&escape_html(t));
                s.push_str("</em>");
            }
            Inline::Code(t) => {
                s.push_str("<code>");
                s.push_str(&escape_html(t));
                s.push_str("</code>");
            }
            Inline::Ref(target) => {
                s.push_str(&format!(
                    "<a href=\"#{t}\">#{t}</a>",
                    t = escape_attr(target)
                ));
            }
        }
    }
    s
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    #[test]
    fn heading_html() {
        let doc = parse("@h1 Hello [#x]\n").unwrap();
        assert_eq!(to_html(&doc), "<h1 id=\"x\">Hello</h1>\n");
    }

    #[test]
    fn paragraph_with_emphasis() {
        let doc = parse("@p hello *world*\n").unwrap();
        assert_eq!(to_html(&doc), "<p>hello <strong>world</strong></p>\n");
    }

    #[test]
    fn code_html_classed() {
        let doc = parse("@code lang=rust\n~~~\nfn x(){}\n~~~\n").unwrap();
        let html = to_html(&doc);
        assert!(html.contains("class=\"language-rust\""));
    }
}
