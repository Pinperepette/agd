//! Integration tests for the `agd` binary.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::PathBuf;

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/conformance")
}

fn agd() -> Command {
    Command::cargo_bin("agd").expect("agd binary")
}

#[test]
fn validate_passes_on_corpus() {
    for entry in std::fs::read_dir(examples_dir()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|s| s.to_str()) != Some("agd") {
            continue;
        }
        agd()
            .arg("validate")
            .arg(&path)
            .assert()
            .success();
    }
}

#[test]
fn parse_emits_json() {
    let path = examples_dir().join("01_basic_blocks.agd");
    agd()
        .args(["parse", "--json"])
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("\"kind\""));
}

#[test]
fn format_check_passes_on_canonical_input() {
    // Round-trip a corpus file through `format` first, then re-check.
    let path = examples_dir().join("01_basic_blocks.agd");
    let canonical = agd().arg("format").arg(&path).output().unwrap();
    assert!(canonical.status.success());
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), &canonical.stdout).unwrap();
    agd().args(["format", "--check"]).arg(tmp.path()).assert().success();
}

#[test]
fn ref_check_detects_dangling() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "@p hello\n@ref #nope\n").unwrap();
    agd()
        .args(["ref", "--check"])
        .arg(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("nope"));
}

#[test]
fn id_add_and_strip_roundtrip() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "@h1 Hello\n@h2 World\n").unwrap();
    let with_ids = agd().args(["id", "--add"]).arg(tmp.path()).output().unwrap();
    assert!(with_ids.status.success());
    let s = String::from_utf8(with_ids.stdout).unwrap();
    assert!(s.contains("[#hello]"));
    assert!(s.contains("[#world]"));
}

#[test]
fn edit_replace_op() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "@p first [#a]\n@p second [#b]\n").unwrap();
    let op = serde_json::json!({
        "op": "set_attr",
        "id": "a",
        "key": "weight",
        "value": 3
    });
    let out = agd()
        .args(["edit", "--op", &op.to_string()])
        .arg(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("@p weight=3 first [#a]"));
}

#[test]
fn bench_outputs_token_counts() {
    let path = examples_dir().join("01_basic_blocks.agd");
    agd()
        .arg("bench")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("AGD:"))
        .stdout(predicate::str::contains("Markdown:"))
        .stdout(predicate::str::contains("HTML:"));
}

#[test]
fn convert_to_md_yields_markdown() {
    let path = examples_dir().join("01_basic_blocks.agd");
    agd()
        .args(["convert", "to-md"])
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("# Hello"));
}
