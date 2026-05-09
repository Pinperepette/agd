//! Conformance corpus runner.
//!
//! For each `.agd` file in `tests/conformance/`, parse it and snapshot
//! the resulting AST as JSON. The paired `.json` file is the expected
//! output. To regenerate after a deliberate change, set `INSTA_UPDATE=1`
//! and run `cargo test`.

use std::fs;
use std::path::{Path, PathBuf};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/conformance")
}

fn collect_inputs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(corpus_dir())
        .expect("conformance dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("agd"))
        .collect();
    v.sort();
    v
}

fn pretty_json(value: &impl serde::Serialize) -> String {
    let mut s = serde_json::to_string_pretty(value).unwrap();
    s.push('\n');
    s
}

fn check_or_update(path: &Path, actual: &str) {
    let want_path = path.with_extension("json");
    let update = std::env::var_os("AGD_UPDATE_FIXTURES").is_some();
    if update || !want_path.exists() {
        fs::write(&want_path, actual).expect("write fixture");
        eprintln!("(updated {})", want_path.display());
        return;
    }
    let want = fs::read_to_string(&want_path).expect("read fixture");
    if want != actual {
        let diff_summary = format!(
            "\n--- expected ({}):\n{}\n--- actual:\n{}",
            want_path.display(),
            want,
            actual
        );
        panic!("conformance mismatch for {}:{diff_summary}", path.display());
    }
}

#[test]
fn corpus_passes() {
    let inputs = collect_inputs();
    assert!(!inputs.is_empty(), "no conformance inputs found in {}", corpus_dir().display());
    for path in inputs {
        let src = fs::read_to_string(&path).expect("read input");
        let mut doc = agd::parse(&src).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        // Stability: re-parse the canonical serialization → must equal the original AST.
        let canon = agd::serialize(&doc);
        let doc2 = agd::parse(&canon).unwrap_or_else(|e| panic!("{} canon reparse: {e}", path.display()));
        assert_eq!(doc, doc2, "canonical reparse drift for {}", path.display());
        // Compare AST JSON to fixture (spans stripped → resilient to whitespace).
        doc.reset_spans();
        let actual = pretty_json(&doc);
        check_or_update(&path, &actual);
    }
}
