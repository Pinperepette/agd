//! AST types for AGD documents. These are the canonical in-memory
//! representation produced by the parser and consumed by the serializer.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A parsed AGD document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Document {
    pub blocks: Vec<Block>,
}

/// A single top-level block. `span` is incidental source-location
/// metadata and is **not** part of structural equality — two blocks
/// with identical kind/attrs/id/content compare equal regardless of
/// where they came from in the source.
#[derive(Debug, Clone, Eq, Serialize, Deserialize)]
pub struct Block {
    pub kind: BlockKind,
    /// Attributes appearing on the block-start line, sorted by key for canonical form.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: BTreeMap<String, AttrValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub content: BlockContent,
    #[serde(default, skip_serializing_if = "Span::is_default")]
    pub span: Span,
}

impl PartialEq for Block {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.attrs == other.attrs
            && self.id == other.id
            && self.content == other.content
    }
}

/// Block tag. Built-in tags use lowercase ASCII; custom tags MUST be `x-`
/// prefixed per spec section 4.2.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlockKind(pub String);

impl BlockKind {
    pub const META: &'static str = "meta";
    pub const H1: &'static str = "h1";
    pub const H2: &'static str = "h2";
    pub const H3: &'static str = "h3";
    pub const H4: &'static str = "h4";
    pub const P: &'static str = "p";
    pub const UL: &'static str = "ul";
    pub const OL: &'static str = "ol";
    pub const CODE: &'static str = "code";
    pub const QUOTE: &'static str = "quote";
    pub const RAW: &'static str = "raw";
    pub const REF: &'static str = "ref";
    pub const INCLUDE: &'static str = "include";
    pub const COMMENT: &'static str = "!";
    pub const TABLE: &'static str = "table";

    pub const BUILTINS: &'static [&'static str] = &[
        Self::META, Self::H1, Self::H2, Self::H3, Self::H4,
        Self::P, Self::UL, Self::OL, Self::CODE, Self::QUOTE,
        Self::RAW, Self::REF, Self::INCLUDE, Self::COMMENT, Self::TABLE,
    ];

    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_builtin(&self) -> bool {
        Self::BUILTINS.contains(&self.0.as_str())
    }

    pub fn is_custom(&self) -> bool {
        self.0.starts_with("x-")
    }

    pub fn is_heading(&self) -> bool {
        matches!(self.0.as_str(), "h1" | "h2" | "h3" | "h4")
    }

    pub fn is_listish(&self) -> bool {
        matches!(self.0.as_str(), "ul" | "ol" | "quote")
    }

    pub fn is_fenced(&self) -> bool {
        matches!(self.0.as_str(), "code" | "raw")
    }

    pub fn takes_inline(&self) -> bool {
        self.is_heading() || self.0 == "p" || self.0 == "ref"
    }
}

impl std::fmt::Display for BlockKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Body of a block. Determined by `BlockKind`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum BlockContent {
    /// h1-h4, p, ref — single line of inline content
    Inline(Vec<Inline>),
    /// ul, ol, quote — each entry is one line of inline content
    Items(Vec<Vec<Inline>>),
    /// code, raw — verbatim text between fences
    Fenced(String),
    /// meta and bodyless blocks
    Empty,
}

impl BlockContent {
    pub fn as_inline(&self) -> Option<&[Inline]> {
        match self {
            BlockContent::Inline(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_items(&self) -> Option<&[Vec<Inline>]> {
        match self {
            BlockContent::Items(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_fenced(&self) -> Option<&str> {
        match self {
            BlockContent::Fenced(s) => Some(s),
            _ => None,
        }
    }
}

/// Inline content within a block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "text", rename_all = "lowercase")]
pub enum Inline {
    Text(String),
    Bold(String),
    Italic(String),
    Code(String),
    /// `@ref #id` resolved at parse time but kept as a node for round-tripping.
    Ref(String),
}

impl Inline {
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text(s.into())
    }

    pub fn as_plain(&self) -> &str {
        match self {
            Inline::Text(s) | Inline::Bold(s) | Inline::Italic(s) | Inline::Code(s) | Inline::Ref(s) => s,
        }
    }
}

/// Byte-offset span into the original source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start: start as u32, end: end as u32 }
    }
    pub fn range(&self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }
    pub fn is_default(&self) -> bool {
        self.start == 0 && self.end == 0
    }
}

/// Attribute value. Numbers without a decimal point are parsed as `Int`.
/// Quoted values are always `Str`. `true` / `false` (unquoted) are `Bool`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AttrValue {
    Bool(bool),
    Int(i64),
    Str(String),
}

impl AttrValue {
    pub fn as_str(&self) -> Option<&str> {
        if let AttrValue::Str(s) = self { Some(s) } else { None }
    }
    pub fn as_int(&self) -> Option<i64> {
        if let AttrValue::Int(n) = self { Some(*n) } else { None }
    }
    pub fn as_bool(&self) -> Option<bool> {
        if let AttrValue::Bool(b) = self { Some(*b) } else { None }
    }
}

impl From<&str> for AttrValue {
    fn from(s: &str) -> Self { AttrValue::Str(s.to_owned()) }
}
impl From<String> for AttrValue {
    fn from(s: String) -> Self { AttrValue::Str(s) }
}
impl From<i64> for AttrValue {
    fn from(n: i64) -> Self { AttrValue::Int(n) }
}
impl From<bool> for AttrValue {
    fn from(b: bool) -> Self { AttrValue::Bool(b) }
}

impl Document {
    pub fn new() -> Self { Self::default() }

    /// Find the first block with the given ID.
    pub fn find(&self, id: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| b.id.as_deref() == Some(id))
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut Block> {
        self.blocks.iter_mut().find(|b| b.id.as_deref() == Some(id))
    }

    pub fn position(&self, id: &str) -> Option<usize> {
        self.blocks.iter().position(|b| b.id.as_deref() == Some(id))
    }

    /// Collect all IDs in document order.
    pub fn ids(&self) -> Vec<&str> {
        self.blocks.iter().filter_map(|b| b.id.as_deref()).collect()
    }

    /// Zero out every block's span (useful for stable snapshots / diffs).
    pub fn reset_spans(&mut self) {
        for b in &mut self.blocks {
            b.span = Span::default();
        }
    }

    /// Merge adjacent `Inline::Text` nodes within every block. The parser
    /// always produces a normalized AST; this method brings hand-built or
    /// randomly generated documents into the same shape.
    pub fn normalize(&mut self) {
        for b in &mut self.blocks {
            match &mut b.content {
                BlockContent::Inline(v) => merge_text_runs(v),
                BlockContent::Items(items) => {
                    for line in items {
                        merge_text_runs(line);
                    }
                }
                _ => {}
            }
        }
    }
}

fn merge_text_runs(v: &mut Vec<Inline>) {
    let mut out: Vec<Inline> = Vec::with_capacity(v.len());
    for node in v.drain(..) {
        if let (Some(Inline::Text(prev)), Inline::Text(next)) = (out.last_mut(), &node) {
            prev.push_str(next);
        } else {
            out.push(node);
        }
    }
    *v = out;
}

impl Block {
    pub fn new(kind: impl Into<String>, content: BlockContent) -> Self {
        Self {
            kind: BlockKind::new(kind),
            attrs: BTreeMap::new(),
            id: None,
            content,
            span: Span::default(),
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn with_attr(mut self, key: impl Into<String>, value: impl Into<AttrValue>) -> Self {
        self.attrs.insert(key.into(), value.into());
        self
    }
}
