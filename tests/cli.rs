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
fn edit_in_place_refuses_non_roundtripping_edit() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let original = "@p first [#a]\n";
    std::fs::write(tmp.path(), original).unwrap();
    // A plain Text run holding a matched delimiter pair: apply() accepts it (it
    // is not a *styled* run), but it serializes to `a*b*c`, which re-parses as
    // Text+Bold+Text — a different document. The in-place write guard must
    // refuse (non-zero exit) and leave the file byte-for-byte untouched.
    let op = serde_json::json!({
        "op": "replace",
        "id": "a",
        "with": {
            "kind": "p",
            "id": "a",
            "content": { "type": "inline", "value": [{ "kind": "text", "text": "a*b*c" }] }
        }
    });
    let out = agd()
        .args(["edit", "--op", &op.to_string(), "--in-place"])
        .arg(tmp.path())
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "expected non-zero exit, got success; stdout={:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    let after = std::fs::read_to_string(tmp.path()).unwrap();
    assert_eq!(after, original, "file must be left untouched when the edit is refused");
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

#[test]
fn backlinks_lists_inbound_references_via_refs_attr() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        tmp.path(),
        "@p target body [#target]\n\n@x-note refs=\"#target\" [#citing]\n~~~\nbody\n~~~\n\n@p unrelated [#other]\n",
    )
    .unwrap();
    let out = agd()
        .args(["backlinks"])
        .arg(tmp.path())
        .arg("#target")
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("citing"), "expected `citing` in: {s}");
    assert!(!s.contains("other"), "unrelated block leaked: {s}");
    assert!(!s.contains("target\t"), "target should not list itself: {s}");
}

#[test]
fn backlinks_empty_when_no_inbound_refs() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "@p one [#a]\n@p two [#b]\n").unwrap();
    let out = agd()
        .args(["backlinks"])
        .arg(tmp.path())
        .arg("#a")
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.trim().is_empty(), "expected empty output, got: {s}");
}

#[test]
fn backlinks_accepts_id_without_hash() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        tmp.path(),
        "@p body [#t]\n\n@x-note refs=\"#t\" [#c]\n~~~\n~~~\n",
    )
    .unwrap();
    agd()
        .args(["backlinks"])
        .arg(tmp.path())
        .arg("t")
        .assert()
        .success()
        .stdout(predicate::str::contains("c"));
}

#[test]
fn get_with_backlinks_appends_inbound() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        tmp.path(),
        "@p target body [#target]\n\n@x-note refs=\"#target\" [#citing]\n~~~\nbody\n~~~\n",
    )
    .unwrap();
    let out = agd()
        .args(["get", "--with-backlinks"])
        .arg(tmp.path())
        .arg("#target")
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("[#target]"), "missing requested block: {s}");
    assert!(s.contains("[#citing]"), "missing inbound: {s}");
}

#[test]
fn get_follow_refs_one_hop() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        tmp.path(),
        "@p root [#root]\n\n@x-note refs=\"#root\" [#mid]\n~~~\nm\n~~~\n\n@x-note refs=\"#mid\" [#leaf]\n~~~\nl\n~~~\n",
    )
    .unwrap();
    let out = agd()
        .args(["get", "--follow-refs"])
        .arg(tmp.path())
        .arg("#leaf")
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("[#leaf]"));
    assert!(s.contains("[#mid]"), "depth=1 should reach mid: {s}");
    assert!(!s.contains("[#root]"), "depth=1 should NOT reach root: {s}");
}

#[test]
fn get_follow_refs_full_chain() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        tmp.path(),
        "@p root [#root]\n\n@x-note refs=\"#root\" [#mid]\n~~~\nm\n~~~\n\n@x-note refs=\"#mid\" [#leaf]\n~~~\nl\n~~~\n",
    )
    .unwrap();
    let out = agd()
        .args(["get", "--follow-refs", "--depth", "5"])
        .arg(tmp.path())
        .arg("#leaf")
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("[#leaf]"));
    assert!(s.contains("[#mid]"));
    assert!(s.contains("[#root]"), "depth=5 should reach root: {s}");
}

#[test]
fn get_follow_refs_handles_cycles() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        tmp.path(),
        "@x-note refs=\"#b\" [#a]\n~~~\na\n~~~\n\n@x-note refs=\"#a\" [#b]\n~~~\nb\n~~~\n",
    )
    .unwrap();
    let out = agd()
        .args(["get", "--follow-refs", "--depth", "10"])
        .arg(tmp.path())
        .arg("#a")
        .output()
        .unwrap();
    assert!(out.status.success(), "should not stack overflow on cycles");
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("[#a]") && s.contains("[#b]"));
    assert_eq!(s.matches("[#a]").count(), 1, "no duplicates: {s}");
    assert_eq!(s.matches("[#b]").count(), 1, "no duplicates: {s}");
}

#[test]
fn get_combines_follow_refs_and_backlinks() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        tmp.path(),
        "@p root [#root]\n\n@x-note refs=\"#root\" [#mid]\n~~~\nm\n~~~\n\n@x-note refs=\"#mid\" [#cite-mid]\n~~~\nc\n~~~\n",
    )
    .unwrap();
    let out = agd()
        .args(["get", "--follow-refs", "--depth", "2", "--with-backlinks"])
        .arg(tmp.path())
        .arg("#mid")
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("[#mid]"));
    assert!(s.contains("[#root]"), "follow-refs should reach root: {s}");
    assert!(s.contains("[#cite-mid]"), "backlinks should pick up citer: {s}");
}

#[test]
fn backlinks_handles_multi_target_refs_attr() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        tmp.path(),
        "@p first [#a]\n\n@p second [#b]\n\n@x-note refs=\"#a, #b\" [#citing]\n~~~\nbody\n~~~\n",
    )
    .unwrap();
    let out_a = agd().args(["backlinks"]).arg(tmp.path()).arg("#a").output().unwrap();
    assert!(out_a.status.success());
    assert!(String::from_utf8(out_a.stdout).unwrap().contains("citing"));
    let out_b = agd().args(["backlinks"]).arg(tmp.path()).arg("#b").output().unwrap();
    assert!(out_b.status.success());
    assert!(String::from_utf8(out_b.stdout).unwrap().contains("citing"));
}
