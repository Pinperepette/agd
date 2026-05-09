//! Property-based roundtrip tests: generate a random Document, serialize
//! it to canonical AGD, re-parse, and assert structural equality.

use std::collections::BTreeMap;

use agd::ast::{AttrValue, Block, BlockContent, BlockKind, Document, Inline};
use proptest::prelude::*;

fn ident_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z_][a-zA-Z0-9_-]{0,12}".prop_map(String::from)
}

fn plain_text_strategy() -> impl Strategy<Value = String> {
    // Avoid every character that could trip the inline parser or block lexer.
    "[a-zA-Z0-9 .,;:!?+/=()'\"\\-]{0,40}".prop_map(|s| s.trim().to_string())
        .prop_filter("non-empty", |s| !s.is_empty())
}

fn run_text_strategy() -> impl Strategy<Value = String> {
    // Used inside *bold*/_italic_/`code` runs — additionally exclude the delimiters themselves.
    "[a-zA-Z0-9 .,;:+/=()'\\-]{1,30}".prop_map(|s| s.trim().to_string())
        .prop_filter("non-empty", |s| !s.is_empty())
}

fn inline_strategy() -> impl Strategy<Value = Inline> {
    // Inline::Ref deliberately excluded — v0.1 only supports refs at block level.
    prop_oneof![
        plain_text_strategy().prop_map(Inline::Text),
        run_text_strategy().prop_map(Inline::Bold),
        run_text_strategy().prop_map(Inline::Italic),
        run_text_strategy().prop_map(Inline::Code),
    ]
}

fn inlines_strategy() -> impl Strategy<Value = Vec<Inline>> {
    prop::collection::vec(inline_strategy(), 1..4)
}

fn attr_value_strategy() -> impl Strategy<Value = AttrValue> {
    prop_oneof![
        any::<bool>().prop_map(AttrValue::Bool),
        (-1000i64..1000).prop_map(AttrValue::Int),
        plain_text_strategy().prop_map(AttrValue::Str),
    ]
}

fn attrs_strategy() -> impl Strategy<Value = BTreeMap<String, AttrValue>> {
    prop::collection::btree_map(ident_strategy(), attr_value_strategy(), 0..3)
}

fn fence_text_strategy() -> impl Strategy<Value = String> {
    // Verbatim fence body — must NOT contain a line equal to `~~~`.
    "[a-zA-Z0-9_ \\.,;:!?+/=(){}'\"\n\\-]{0,80}"
        .prop_map(|s| {
            s.lines()
                .filter(|l| l.trim() != "~~~")
                .collect::<Vec<_>>()
                .join("\n")
        })
}

fn block_strategy() -> impl Strategy<Value = Block> {
    prop_oneof![
        // headings, paragraphs (inline-bearing)
        (
            prop::sample::select(vec!["h1", "h2", "h3", "h4", "p"]),
            inlines_strategy(),
            prop::option::of(ident_strategy()),
        )
            .prop_map(|(tag, inl, id)| Block {
                kind: BlockKind::new(tag),
                attrs: BTreeMap::new(),
                id,
                content: BlockContent::Inline(inl),
                span: Default::default(),
            }),
        // lists
        (
            prop::sample::select(vec!["ul", "ol"]),
            prop::collection::vec(inlines_strategy(), 1..5),
            prop::option::of(ident_strategy()),
        )
            .prop_map(|(tag, items, id)| Block {
                kind: BlockKind::new(tag),
                attrs: BTreeMap::new(),
                id,
                content: BlockContent::Items(items),
                span: Default::default(),
            }),
        // quotes
        (
            prop::collection::vec(inlines_strategy(), 1..4),
            prop::option::of(ident_strategy()),
            attrs_strategy(),
        )
            .prop_map(|(items, id, attrs)| Block {
                kind: BlockKind::new("quote"),
                attrs,
                id,
                content: BlockContent::Items(items),
                span: Default::default(),
            }),
        // fenced code/raw
        (
            prop::sample::select(vec!["code", "raw"]),
            fence_text_strategy(),
            prop::option::of(ident_strategy()),
            attrs_strategy(),
        )
            .prop_map(|(tag, body, id, attrs)| Block {
                kind: BlockKind::new(tag),
                attrs,
                id,
                content: BlockContent::Fenced(body),
                span: Default::default(),
            }),
        // meta / include — empty, attrs only
        (
            prop::sample::select(vec!["meta", "include"]),
            attrs_strategy(),
        )
            .prop_map(|(tag, attrs)| Block {
                kind: BlockKind::new(tag),
                attrs,
                id: None,
                content: BlockContent::Empty,
                span: Default::default(),
            }),
        // ref — single inline ref
        (ident_strategy(), prop::option::of(ident_strategy())).prop_map(|(target, id)| Block {
            kind: BlockKind::new("ref"),
            attrs: BTreeMap::new(),
            id,
            content: BlockContent::Inline(vec![Inline::Ref(target)]),
            span: Default::default(),
        }),
    ]
}

fn document_strategy() -> impl Strategy<Value = Document> {
    prop::collection::vec(block_strategy(), 0..6).prop_map(|mut blocks| {
        // Ensure ID uniqueness (parser would otherwise reject duplicates).
        let mut seen = std::collections::HashSet::new();
        for b in &mut blocks {
            if let Some(id) = &b.id {
                if !seen.insert(id.clone()) {
                    b.id = None;
                }
            }
        }
        let mut doc = Document::with_blocks(blocks);
        doc.normalize();
        doc
    })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        max_shrink_iters: 4096,
        ..ProptestConfig::default()
    })]

    #[test]
    fn roundtrip_canonical(doc in document_strategy()) {
        let canonical = agd::serialize(&doc);
        let parsed = agd::parse(&canonical)
            .map_err(|e| TestCaseError::fail(format!("parse failed: {e}\n--- input ---\n{canonical}")))?;
        prop_assert_eq!(doc, parsed);
    }

    #[test]
    fn canonicalize_is_idempotent(doc in document_strategy()) {
        let once = agd::serialize(&doc);
        let twice = agd::canonicalize(&once)
            .map_err(|e| TestCaseError::fail(format!("canonicalize failed: {e}\n--- input ---\n{once}")))?;
        prop_assert_eq!(once, twice);
    }
}
