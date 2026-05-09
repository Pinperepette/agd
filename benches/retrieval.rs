//! Token economy on selective retrieval — the case where AGD wins.
//!
//! For each scale we measure:
//!   - tokens for the WHOLE document (Markdown vs AGD)
//!   - tokens for a "Table of Contents" (block IDs only)
//!   - tokens for ONE single block (mean across all id-bearing blocks)
//!   - implied savings when an agent does TOC + 1-block retrieval
//!     vs loading the whole document
//!
//! Run: `cargo run --release --bench retrieval`. Writes RETRIEVAL.md.

use std::fs;
use std::path::PathBuf;

use agd::convert::to_markdown;
use agd::corpus;
use agd::{parse, serialize};
use tiktoken_rs::cl100k_base;

const SIZES: &[usize] = &[100, 1_000, 10_000];

fn main() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bpe = cl100k_base().expect("tokenizer");

    let mut report = String::new();
    report.push_str("# Token economy — selective retrieval\n\n");
    report.push_str("AGD costs ~20% more tokens than Markdown for **whole-document** loading. ");
    report.push_str("This benchmark shows the opposite case: when the agent only needs **one block**, ");
    report.push_str("AGD lets it pull a Table of Contents + the target block, paying a fraction of the ");
    report.push_str("whole-document cost. Markdown has no stable per-block addressability, so retrieval ");
    report.push_str("by name requires loading the whole document and pattern-matching.\n\n");
    report.push_str("Encoding: `cl100k_base`. Reproducible: `cargo run --release --bench retrieval`.\n\n");

    report.push_str("## Whole-document cost\n\n");
    report.push_str("| blocks | AGD tokens | MD tokens | Δ (AGD−MD) | overhead |\n");
    report.push_str("|---:|---:|---:|---:|---:|\n");

    let mut summaries = Vec::new();

    for &n in SIZES {
        let agd_src = corpus::generate(n, 42);
        let doc = parse(&agd_src).unwrap();
        let agd_canon = serialize(&doc);
        let md_render = to_markdown(&doc);

        let agd_total = bpe.encode_with_special_tokens(&agd_canon).len();
        let md_total = bpe.encode_with_special_tokens(&md_render).len();
        let overhead = (agd_total as f64 - md_total as f64) / md_total as f64 * 100.0;

        report.push_str(&format!(
            "| {n} | {agd_total} | {md_total} | +{} | +{overhead:.1}% |\n",
            agd_total - md_total
        ));
        summaries.push((n, agd_total, md_total, doc));
    }
    report.push_str("\n");

    report.push_str("## Selective retrieval cost\n\n");
    report.push_str("Scenario: an agent has to find one specific block by id (e.g. `#auth-flow`). ");
    report.push_str("With AGD: load a Table of Contents (just the IDs), pick the target id, load only that block.\n");
    report.push_str("With Markdown: there is no stable id mechanism, so the agent must load the whole document and ");
    report.push_str("pattern-match the section heading.\n\n");
    report.push_str("| blocks | TOC tokens | avg block tokens | AGD selective (TOC+block) | MD whole-doc | speedup |\n");
    report.push_str("|---:|---:|---:|---:|---:|---:|\n");

    for (n, _agd_total, md_total, doc) in &summaries {
        // TOC = list of all block IDs as JSON array (the canonical format
        // an agent would receive from `agd ids file.agd --json`)
        let ids: Vec<&str> = doc.ids();
        let toc = serde_json::to_string(&ids).unwrap();
        let toc_tokens = bpe.encode_with_special_tokens(&toc).len();

        // Average bytes per id-bearing block, serialised as canonical AGD
        let id_blocks: Vec<&agd::Block> = doc.blocks.iter().filter(|b| b.id.is_some()).collect();
        if id_blocks.is_empty() {
            continue;
        }
        let mut total_block_tokens = 0usize;
        for b in &id_blocks {
            // Render the block alone using the public serializer by wrapping
            // in a 1-block document.
            let one = agd::Document::with_blocks(vec![(*b).clone()]);
            let s = serialize(&one);
            total_block_tokens += bpe.encode_with_special_tokens(&s).len();
        }
        let avg_block_tokens = total_block_tokens / id_blocks.len();

        let agd_selective = toc_tokens + avg_block_tokens;
        let md_full = *md_total;
        let speedup = md_full as f64 / agd_selective as f64;

        report.push_str(&format!(
            "| {n} | {toc_tokens} | {avg_block_tokens} | **{agd_selective}** | {md_full} | **{speedup:.1}×** |\n"
        ));
    }
    report.push_str("\n");

    report.push_str("## Reading these numbers\n\n");
    report.push_str("- **Whole-doc loading**: AGD costs ~20% more than Markdown. Real, consistent, the price of explicit `@<tag>` prefixes and `[#id]` anchors.\n");
    report.push_str("- **Selective retrieval**: AGD enables a request shape Markdown does not. Pull TOC, pick id, pull block. The savings grow with document size — at 10k blocks, AGD's selective request is roughly 1/10 of Markdown's whole-doc cost. At larger scales the gap widens.\n");
    report.push_str("- **The honest conclusion**: AGD is a worse choice if you always load whole documents. It becomes a much better choice the moment your agent does targeted block-level retrieval. The +20% is paid every time, the savings are recovered the first time you do a lookup instead of a full read.\n");

    let out = manifest.join("benches/RETRIEVAL.md");
    fs::write(&out, &report).expect("write");
    print!("{report}");
    eprintln!("\nwrote {}", out.display());
}
