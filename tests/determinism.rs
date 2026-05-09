//! Determinism tests — same input must always produce the same AST,
//! and the canonical serialiser must be a fixed point under iteration.
//!
//! These properties are what HTML claims (every spec-compliant parser
//! produces the same DOM) and what Markdown loses (CommonMark vs GFM
//! vs MDX disagree on edge cases). AGD is a single implementation, so
//! the strongest determinism property we can verify here is *self-
//! determinism*: parse(serialize(parse(x))) == parse(x), repeatedly,
//! across many runs.

use agd::corpus;

#[test]
fn parse_is_pure_function() {
    // Same input parsed N times must produce byte-identical AST JSON.
    let src = corpus::generate(500, 17);
    let mut last: Option<String> = None;
    for i in 0..50 {
        let doc = agd::parse(&src).expect("parse");
        let json = serde_json::to_string(&doc).unwrap();
        if let Some(prev) = &last {
            assert_eq!(prev, &json, "drift on iteration {i}");
        }
        last = Some(json);
    }
}

#[test]
fn canonicalize_is_fixed_point() {
    // canonicalize ∘ canonicalize == canonicalize, repeated.
    for seed in [1u64, 7, 42, 99, 2026] {
        for n in [50usize, 200, 1000] {
            let src = corpus::generate(n, seed);
            let c1 = agd::canonicalize(&src).expect("canon 1");
            let c2 = agd::canonicalize(&c1).expect("canon 2");
            let c3 = agd::canonicalize(&c2).expect("canon 3");
            assert_eq!(c1, c2, "fixed point fails at seed={seed} n={n} (1↔2)");
            assert_eq!(c2, c3, "fixed point fails at seed={seed} n={n} (2↔3)");
        }
    }
}

#[test]
fn ast_equality_unaffected_by_whitespace_runs() {
    // Inserting extra blank lines between blocks must not change the AST.
    let base = "@h1 Hello [#a]\n\n@p body [#b]\n\n@h2 Sub\n\n@p more\n";
    let extra = "@h1 Hello [#a]\n\n\n\n\n@p body [#b]\n\n\n\n@h2 Sub\n\n\n\n@p more\n";
    let d1 = agd::parse(base).unwrap();
    let d2 = agd::parse(extra).unwrap();
    assert_eq!(d1, d2);
}

#[test]
fn id_uniqueness_enforced_globally() {
    let src = "@h1 First [#x]\n@p Body\n@h2 Later [#x]\n";
    assert!(matches!(
        agd::parse(src),
        Err(agd::AgdError::DuplicateId { .. })
    ));
}

#[test]
fn malformed_tag_rejected_consistently() {
    // Any unknown tag without `x-` prefix must be rejected.
    for tag in &["@unknown", "@xx-missingdash", "@123abc", "@H1"] {
        let src = format!("{tag} content\n");
        let result = agd::parse(&src);
        assert!(
            result.is_err(),
            "expected error for `{tag}`, got: {:?}",
            result
        );
    }
}

#[test]
fn x_prefixed_custom_tags_accepted() {
    let src = "@x-diagram type=flow [#d1]\n@x-equation [#e1]\n";
    let doc = agd::parse(src).unwrap();
    assert_eq!(doc.blocks.len(), 2);
    assert!(doc.blocks.iter().all(|b| b.kind.is_custom()));
}
