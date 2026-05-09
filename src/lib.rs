//! # AGD — Agent Document Format
//!
//! A line-oriented text format optimized for LLM agents. Sits between
//! Markdown (too ambiguous for machine editing) and HTML/XML (too verbose
//! for token-constrained LLM contexts). See `grammar/agd.ebnf` and
//! `spec/AGD-SPEC.agd` for the formal definition.
//!
//! ## Quickstart
//!
//! ```
//! use agd::{parse, serialize};
//!
//! let doc = parse("@h1 Hello [#intro]\n@p World\n").unwrap();
//! assert_eq!(doc.blocks.len(), 2);
//! let canonical = serialize(&doc);
//! ```
//!
//! Stable block IDs (`[#id]`) make multi-agent editing safe — see
//! [`edit::Operation`] for the operation algebra.

pub mod ast;
pub mod convert;
pub mod corpus;
pub mod edit;
pub mod error;
pub mod id;
pub mod index;
pub mod lexer;
pub mod parser;
pub mod serializer;

pub use ast::{AttrValue, Block, BlockContent, BlockKind, Document, Inline, Span};
pub use error::{AgdError, Result};
pub use parser::parse;
pub use serializer::serialize;

/// Convenience: parse then re-serialize. Idempotent on already-canonical input.
pub fn canonicalize(src: &str) -> Result<String> {
    Ok(serialize(&parse(src)?))
}

/// Validate that every `@ref` (block-level and inline) points to an
/// existing block ID. Returns the first dangling reference, if any.
pub fn check_refs(doc: &Document) -> Result<()> {
    use std::collections::HashSet;
    let ids: HashSet<&str> = doc.blocks.iter().filter_map(|b| b.id.as_deref()).collect();
    for block in &doc.blocks {
        let inlines: Vec<&Inline> = match &block.content {
            BlockContent::Inline(v) => v.iter().collect(),
            BlockContent::Items(items) => items.iter().flatten().collect(),
            _ => Vec::new(),
        };
        for inl in inlines {
            if let Inline::Ref(target) = inl {
                if !ids.contains(target.as_str()) {
                    return Err(AgdError::DanglingRef {
                        target: target.clone(),
                        line: 0,
                    });
                }
            }
        }
    }
    Ok(())
}
