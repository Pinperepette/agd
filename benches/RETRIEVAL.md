# Token economy — selective retrieval

AGD costs ~20% more tokens than Markdown for **whole-document** loading. This benchmark shows the opposite case: when the agent only needs **one block**, AGD lets it pull a Table of Contents + the target block, paying a fraction of the whole-document cost. Markdown has no stable per-block addressability, so retrieval by name requires loading the whole document and pattern-matching.

Encoding: `cl100k_base`. Reproducible: `cargo run --release --bench retrieval`.

## Whole-document cost

| blocks | AGD tokens | MD tokens | Δ (AGD−MD) | overhead |
|---:|---:|---:|---:|---:|
| 100 | 2532 | 2074 | +458 | +22.1% |
| 1000 | 24589 | 20502 | +4087 | +19.9% |
| 10000 | 256338 | 217170 | +39168 | +18.0% |

## Selective retrieval cost

Scenario: an agent has to find one specific block by id (e.g. `#auth-flow`). With AGD: load a Table of Contents (just the IDs), pick the target id, load only that block.
With Markdown: there is no stable id mechanism, so the agent must load the whole document and pattern-match the section heading.

| blocks | TOC tokens | avg block tokens | AGD selective (TOC+block) | MD whole-doc | speedup |
|---:|---:|---:|---:|---:|---:|
| 100 | 194 | 37 | **231** | 2074 | **9.0×** |
| 1000 | 1682 | 38 | **1720** | 20502 | **11.9×** |
| 10000 | 14860 | 42 | **14902** | 217170 | **14.6×** |

## Reading these numbers

- **Whole-doc loading**: AGD costs ~20% more than Markdown. Real, consistent, the price of explicit `@<tag>` prefixes and `[#id]` anchors.
- **Selective retrieval**: AGD enables a request shape Markdown does not. Pull TOC, pick id, pull block. The savings grow with document size — at 10k blocks, AGD's selective request is roughly 1/10 of Markdown's whole-doc cost. At larger scales the gap widens.
- **The honest conclusion**: AGD is a worse choice if you always load whole documents. It becomes a much better choice the moment your agent does targeted block-level retrieval. The +20% is paid every time, the savings are recovered the first time you do a lookup instead of a full read.
