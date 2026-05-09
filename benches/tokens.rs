//! Token-count comparison benchmark: AGD vs CommonMark vs HTML vs JSON.
//! This is NOT a perf bench — it produces a results report.

use std::fs;
use std::path::PathBuf;

use agd::convert::{to_html, to_markdown};
use agd::{parse, serialize};
use tiktoken_rs::cl100k_base;

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let examples_dir = manifest_dir.join("examples");

    let bpe = cl100k_base().expect("cl100k_base tokenizer");

    let mut entries: Vec<PathBuf> = fs::read_dir(&examples_dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("agd"))
                .collect()
        })
        .unwrap_or_default();
    entries.sort();

    let mut report = String::new();
    report.push_str("# Token benchmark\n\n");
    report.push_str("Encoding: cl100k_base (GPT-3.5/4, Claude-equivalent for comparison).\n\n");
    report.push_str("| File | AGD | Markdown | HTML | JSON | Δ vs MD | Δ vs HTML |\n");
    report.push_str("|---|---:|---:|---:|---:|---:|---:|\n");

    for path in &entries {
        let agd_src = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let doc = match parse(&agd_src) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("skip {} ({e})", path.display());
                continue;
            }
        };
        let agd_canon = serialize(&doc);
        let md = to_markdown(&doc);
        let html = to_html(&doc);
        let json = serde_json::to_string(&doc).unwrap();

        let agd_n = bpe.encode_with_special_tokens(&agd_canon).len();
        let md_n = bpe.encode_with_special_tokens(&md).len();
        let html_n = bpe.encode_with_special_tokens(&html).len();
        let json_n = bpe.encode_with_special_tokens(&json).len();

        let delta_md = pct(agd_n as i64 - md_n as i64, md_n as i64);
        let delta_html = pct(agd_n as i64 - html_n as i64, html_n as i64);

        report.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            path.file_name().unwrap().to_string_lossy(),
            agd_n,
            md_n,
            html_n,
            json_n,
            delta_md,
            delta_html,
        ));
    }

    let out = manifest_dir.join("benches").join("RESULTS.md");
    fs::write(&out, &report).expect("write RESULTS.md");
    println!("{report}");
    println!("\nWrote {}", out.display());
}

fn pct(diff: i64, base: i64) -> String {
    if base == 0 {
        return "—".into();
    }
    let p = (diff as f64) / (base as f64) * 100.0;
    if p.abs() < 0.05 {
        "0%".into()
    } else if p > 0.0 {
        format!("+{p:.1}%")
    } else {
        format!("{p:.1}%")
    }
}
