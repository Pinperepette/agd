//! Deterministic synthetic-corpus generator for benchmarking and testing.
//!
//! Given a target block count and a seed, [`generate`] produces an AGD
//! document with a realistic mix of headings, paragraphs, code blocks,
//! lists, quotes, and references. Same `(n, seed)` always produces the
//! same bytes — fully reproducible.
//!
//! ```
//! let agd = agd::corpus::generate(100, 42);
//! let doc = agd::parse(&agd).unwrap();
//! assert!(doc.blocks.len() >= 100);
//! ```

use std::fmt::Write;

const WORDS: &[&str] = &[
    "the", "quick", "brown", "fox", "jumps", "over", "lazy", "dog",
    "machine", "learning", "model", "data", "quality", "reliability", "service",
    "level", "objective", "error", "budget", "team", "scale", "graph", "vector",
    "embedding", "context", "window", "token", "prompt", "agent", "document",
    "stable", "identifier", "block", "structure", "format", "parser", "lexer",
    "grammar", "rule", "fence", "list", "quote", "heading", "paragraph",
    "reference", "metadata", "attribute", "value", "canonical", "idempotent",
    "round", "trip", "byte", "offset", "span", "fixture", "regression",
];

const SENTENCE_PATTERNS: &[&[u8]] = &[
    b"AAAVO",
    b"DAAVAA",
    b"AAVA",
    b"AAVAAA",
    b"VAAAA",
];

/// Generate `n_blocks` of synthetic AGD content with the given seed.
/// The output is always a valid, parseable AGD document. Block kinds
/// are sampled from a realistic distribution so the corpus is not
/// dominated by any single tag.
pub fn generate(n_blocks: usize, seed: u64) -> String {
    let mut rng = Lcg::new(seed.max(1));
    let mut out = String::with_capacity(n_blocks * 80);

    write_meta(&mut out, n_blocks, seed);

    let mut emitted: usize = 1;
    let mut h1_ids: Vec<String> = Vec::new();
    let mut h2_ids: Vec<String> = Vec::new();

    while emitted < n_blocks {
        let r = rng.gen_u32();
        // Distribution: h1 5%, h2 12%, h3 15%, p 32%, ul 10%, ol 5%,
        //               code 10%, quote 8%, ref 3%.
        let kind = match r % 100 {
            0..=4 => "h1",
            5..=16 => "h2",
            17..=31 => "h3",
            32..=63 => "p",
            64..=73 => "ul",
            74..=78 => "ol",
            79..=88 => "code",
            89..=96 => "quote",
            _ => "ref",
        };

        match kind {
            "h1" => {
                let id = format!("h1-{:06}", emitted);
                let title = capitalised_sentence(&mut rng, 4);
                let _ = writeln!(out, "@h1 {title} [#{id}]");
                let _ = writeln!(out);
                h1_ids.push(id);
                emitted += 1;
            }
            "h2" => {
                let id = format!("h2-{:06}", emitted);
                let title = capitalised_sentence(&mut rng, 5);
                let with_id = rng.gen_u32() % 4 != 0; // 75% with ID
                if with_id {
                    let _ = writeln!(out, "@h2 {title} [#{id}]");
                    h2_ids.push(id);
                } else {
                    let _ = writeln!(out, "@h2 {title}");
                }
                let _ = writeln!(out);
                emitted += 1;
            }
            "h3" => {
                let title = capitalised_sentence(&mut rng, 6);
                let _ = writeln!(out, "@h3 {title}");
                emitted += 1;
                let n_paras = (rng.gen_u32() % 3) as usize + 1;
                for _ in 0..n_paras {
                    if emitted >= n_blocks { break; }
                    let _ = writeln!(out, "@p {}", paragraph_text(&mut rng, 2));
                    emitted += 1;
                }
                let _ = writeln!(out);
            }
            "p" => {
                let n = (rng.gen_u32() % 3 + 2) as usize;
                let body = paragraph_text(&mut rng, n);
                let _ = writeln!(out, "@p {body}");
                emitted += 1;
                let _ = writeln!(out);
            }
            "ul" | "ol" => {
                let id = format!("{kind}-{:06}", emitted);
                let with_id = rng.gen_u32() % 2 == 0;
                if with_id {
                    let _ = writeln!(out, "@{kind} [#{id}]");
                } else {
                    let _ = writeln!(out, "@{kind}");
                }
                let n_items = (rng.gen_u32() % 5) as usize + 3;
                for _ in 0..n_items {
                    let _ = writeln!(out, "- {}", capitalised_sentence(&mut rng, 6));
                }
                let _ = writeln!(out);
                emitted += 1;
            }
            "code" => {
                let id = format!("code-{:06}", emitted);
                let lang = ["rust", "python", "go", "json", "shell"][(rng.gen_u32() as usize) % 5];
                let _ = writeln!(out, "@code lang={lang} [#{id}]");
                let _ = writeln!(out, "~~~");
                let n_lines = (rng.gen_u32() % 6) as usize + 3;
                for i in 0..n_lines {
                    let _ = writeln!(
                        out,
                        "// line {i:02} — {}",
                        plain_words(&mut rng, 5)
                    );
                }
                let _ = writeln!(out, "~~~");
                let _ = writeln!(out);
                emitted += 1;
            }
            "quote" => {
                let id = format!("q-{:06}", emitted);
                let with_id = rng.gen_u32() % 3 != 0;
                let src = format!("rfc-{}", rng.gen_u32() % 9000 + 1000);
                if with_id {
                    let _ = writeln!(out, "@quote source=\"{src}\" [#{id}]");
                } else {
                    let _ = writeln!(out, "@quote source=\"{src}\"");
                }
                let n_lines = (rng.gen_u32() % 3) as usize + 2;
                for _ in 0..n_lines {
                    let _ = writeln!(out, "> {}", capitalised_sentence(&mut rng, 8));
                }
                let _ = writeln!(out);
                emitted += 1;
            }
            "ref" => {
                // Only emit a ref to an already-seen ID, otherwise skip.
                let pool: &[String] = if !h2_ids.is_empty() {
                    &h2_ids
                } else if !h1_ids.is_empty() {
                    &h1_ids
                } else {
                    continue;
                };
                let target = &pool[(rng.gen_u32() as usize) % pool.len()];
                let _ = writeln!(out, "@ref #{target}");
                let _ = writeln!(out);
                emitted += 1;
            }
            _ => unreachable!(),
        }
    }

    let _ = writeln!(out, "@! generated by agd::corpus seed={seed} blocks={n_blocks}");
    out
}

fn write_meta(out: &mut String, n_blocks: usize, seed: u64) {
    let _ = writeln!(
        out,
        "@meta corpus=true seed={seed} target_blocks={n_blocks} schema=\"agd-bench-v1\""
    );
    let _ = writeln!(out);
}

fn pick_word<'a>(rng: &mut Lcg, words: &'a [&'a str]) -> &'a str {
    words[(rng.gen_u32() as usize) % words.len()]
}

fn plain_words(rng: &mut Lcg, n: usize) -> String {
    (0..n).map(|_| pick_word(rng, WORDS)).collect::<Vec<_>>().join(" ")
}

fn capitalised_sentence(rng: &mut Lcg, n: usize) -> String {
    let mut s = plain_words(rng, n);
    if let Some(c) = s.get_mut(0..1) {
        c.make_ascii_uppercase();
    }
    s
}

fn paragraph_text(rng: &mut Lcg, sentences: usize) -> String {
    let mut s = String::new();
    for i in 0..sentences {
        if i > 0 {
            s.push(' ');
        }
        let pattern = SENTENCE_PATTERNS[(rng.gen_u32() as usize) % SENTENCE_PATTERNS.len()];
        for (j, b) in pattern.iter().enumerate() {
            if j > 0 {
                s.push(' ');
            }
            let word = match b {
                b'A' | b'V' | b'D' | b'O' => pick_word(rng, WORDS),
                _ => "and",
            };
            if j == 0 {
                let mut w = word.to_string();
                if let Some(c) = w.get_mut(0..1) {
                    c.make_ascii_uppercase();
                }
                s.push_str(&w);
            } else {
                s.push_str(word);
            }
        }
        s.push('.');
    }
    s
}

/// Tiny reproducible LCG. Not cryptographic — used only to make
/// benchmark corpora deterministic without pulling in `rand`.
#[derive(Debug, Clone)]
pub struct Lcg(u64);

impl Lcg {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn gen_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 32) as u32
    }
}

/// Generate the same logical corpus as a synthetic Markdown document.
/// Used to compare AGD vs Markdown parser performance on equivalent
/// content. The output deliberately uses CommonMark-compatible syntax
/// so `pulldown-cmark` can parse it.
pub fn generate_markdown(n_blocks: usize, seed: u64) -> String {
    let mut rng = Lcg::new(seed.max(1));
    let mut out = String::with_capacity(n_blocks * 70);

    let _ = writeln!(out, "<!-- corpus seed={seed} blocks={n_blocks} -->");
    let _ = writeln!(out);

    let mut emitted: usize = 1;
    while emitted < n_blocks {
        let r = rng.gen_u32();
        let kind = match r % 100 {
            0..=4 => "h1",
            5..=16 => "h2",
            17..=31 => "h3",
            32..=63 => "p",
            64..=73 => "ul",
            74..=78 => "ol",
            79..=88 => "code",
            89..=96 => "quote",
            _ => "p",
        };
        match kind {
            "h1" => {
                let _ = writeln!(out, "# {}", capitalised_sentence(&mut rng, 4));
                let _ = writeln!(out);
                emitted += 1;
            }
            "h2" => {
                let _ = writeln!(out, "## {}", capitalised_sentence(&mut rng, 5));
                let _ = writeln!(out);
                emitted += 1;
            }
            "h3" => {
                let _ = writeln!(out, "### {}", capitalised_sentence(&mut rng, 6));
                let _ = writeln!(out);
                emitted += 1;
                for _ in 0..(rng.gen_u32() % 3 + 1) {
                    if emitted >= n_blocks { break; }
                    let _ = writeln!(out, "{}", paragraph_text(&mut rng, 2));
                    let _ = writeln!(out);
                    emitted += 1;
                }
            }
            "p" => {
                let n = (rng.gen_u32() % 3 + 2) as usize;
                let _ = writeln!(out, "{}", paragraph_text(&mut rng, n));
                let _ = writeln!(out);
                emitted += 1;
            }
            "ul" => {
                for _ in 0..(rng.gen_u32() % 5 + 3) {
                    let _ = writeln!(out, "- {}", capitalised_sentence(&mut rng, 6));
                }
                let _ = writeln!(out);
                emitted += 1;
            }
            "ol" => {
                for i in 0..(rng.gen_u32() % 5 + 3) {
                    let _ = writeln!(out, "{}. {}", i + 1, capitalised_sentence(&mut rng, 6));
                }
                let _ = writeln!(out);
                emitted += 1;
            }
            "code" => {
                let lang = ["rust", "python", "go", "json", "shell"][(rng.gen_u32() as usize) % 5];
                let _ = writeln!(out, "```{lang}");
                for i in 0..(rng.gen_u32() % 6 + 3) {
                    let _ = writeln!(out, "// line {i:02} — {}", plain_words(&mut rng, 5));
                }
                let _ = writeln!(out, "```");
                let _ = writeln!(out);
                emitted += 1;
            }
            "quote" => {
                for _ in 0..(rng.gen_u32() % 3 + 2) {
                    let _ = writeln!(out, "> {}", capitalised_sentence(&mut rng, 8));
                }
                let _ = writeln!(out);
                emitted += 1;
            }
            _ => unreachable!(),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_doc_parses() {
        for n in [10, 100, 500] {
            let src = generate(n, 42);
            let doc = crate::parse(&src).unwrap_or_else(|e| panic!("n={n}: {e}"));
            assert!(doc.blocks.len() >= n, "n={n} got {} blocks", doc.blocks.len());
        }
    }

    #[test]
    fn determinism() {
        let a = generate(200, 7);
        let b = generate(200, 7);
        assert_eq!(a, b);
    }

    #[test]
    fn different_seeds_differ() {
        let a = generate(200, 1);
        let b = generate(200, 2);
        assert_ne!(a, b);
    }

    #[test]
    fn markdown_variant_is_valid_md() {
        // smoke test: pulldown-cmark consumes it without panicking
        let md = generate_markdown(100, 11);
        let parser = pulldown_cmark::Parser::new(&md);
        let _events: Vec<_> = parser.collect();
    }
}
