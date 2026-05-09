//! Partial-input behaviour. Truncate a valid document at every byte
//! offset and verify the parser either succeeds or fails cleanly —
//! never panics, never produces undefined behaviour.

use agd::{corpus, parse};

#[test]
fn truncation_never_panics() {
    let src = corpus::generate(200, 11);
    // Sample ~200 truncation points evenly spaced, snapping to safe
    // char boundaries so we don't accidentally cut inside a UTF-8
    // codepoint (which would panic in &str[..cut]).
    let step = (src.len() / 200).max(1);
    let mut clean_failures = 0usize;
    let mut successful_prefixes = 0usize;
    let mut tried = 0usize;
    let mut cut = 0usize;
    while cut <= src.len() {
        if src.is_char_boundary(cut) {
            let prefix = &src[..cut];
            match parse(prefix) {
                Ok(_) => successful_prefixes += 1,
                Err(_) => clean_failures += 1,
            }
            tried += 1;
        }
        cut = cut.saturating_add(step);
    }
    assert!(tried > 50, "expected many truncation points, got {tried}");
    assert!(clean_failures > 0, "no truncation produced an error");
    assert!(
        successful_prefixes > 0,
        "no truncation produced a valid prefix-parse"
    );
}

#[test]
fn unterminated_fence_yields_clean_diagnostic() {
    let src = "@code lang=rust [#x]\n~~~\nfn main() {}\n";
    let err = parse(src).expect_err("must fail");
    assert!(
        matches!(err, agd::AgdError::UnterminatedFence { .. }),
        "wrong error: {err:?}"
    );
}

#[test]
fn unterminated_quoted_attr_yields_diagnostic() {
    let src = "@meta title=\"never closes\n";
    let err = parse(src).expect_err("must fail");
    assert!(
        matches!(err, agd::AgdError::InvalidAttr { .. }),
        "wrong error: {err:?}"
    );
}

#[test]
fn empty_document_is_valid() {
    let doc = parse("").unwrap();
    assert!(doc.blocks.is_empty());
}

#[test]
fn lonely_blank_line_is_valid() {
    let doc = parse("\n\n\n").unwrap();
    assert!(doc.blocks.is_empty());
}

#[test]
fn no_trailing_newline_still_parses() {
    let doc = parse("@h1 No trailing newline").unwrap();
    assert_eq!(doc.blocks.len(), 1);
}

#[test]
fn random_bytes_dont_panic() {
    // Throw arbitrary noise at the parser. We only require that it
    // returns a structured error, never panics.
    let mut rng = corpus::Lcg::new(0xCAFE);
    for _ in 0..200 {
        let len = (rng.gen_u32() as usize) % 256;
        let bytes: Vec<u8> = (0..len).map(|_| (rng.gen_u32() & 0x7f) as u8).collect();
        let s = String::from_utf8_lossy(&bytes).into_owned();
        let _ = parse(&s); // must not panic
    }
}
