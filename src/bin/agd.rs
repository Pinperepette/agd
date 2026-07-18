//! `agd` — command-line driver for the Agent Document toolchain.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

use agd::convert::{from_markdown, to_html, to_markdown};
use agd::edit::Operation;
use agd::id;
use agd::{check_refs, parse, serialize, Document};

#[derive(Parser, Debug)]
#[command(name = "agd", about = "Agent Document — parser, formatter, edit toolchain", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Parse an AGD file and print the AST.
    Parse {
        file: PathBuf,
        /// Emit AST as JSON instead of pretty Debug.
        #[arg(long)]
        json: bool,
    },
    /// Check that an AGD file parses; exit 0 if valid, non-zero otherwise.
    Validate { file: PathBuf },
    /// Re-emit a file in canonical form.
    Format {
        file: PathBuf,
        /// Edit the file in place.
        #[arg(short, long)]
        in_place: bool,
        /// Exit non-zero if the file is not already canonical (no writes).
        #[arg(long)]
        check: bool,
    },
    /// Convert between AGD, Markdown, and HTML.
    Convert {
        #[command(subcommand)]
        kind: ConvertKind,
    },
    /// Run a token count benchmark on an AGD file (writes to stdout).
    Bench { file: PathBuf },
    /// Manage block IDs.
    Id {
        file: PathBuf,
        #[arg(long, conflicts_with = "strip")]
        add: bool,
        #[arg(long)]
        strip: bool,
        #[arg(short, long)]
        in_place: bool,
    },
    /// Apply an edit operation supplied as a JSON object. See OPERATION
    /// SCHEMA below for the full grammar with examples.
    #[command(long_about = "\
Apply an edit operation supplied as a JSON object.

OPERATION SCHEMA (six variants, all keyed by stable block id):

  {\"op\":\"replace\",       \"id\":\"X\", \"with\":  {block}}
  {\"op\":\"insert_after\",  \"id\":\"X\", \"block\": {block}}
  {\"op\":\"insert_before\", \"id\":\"X\", \"block\": {block}}
  {\"op\":\"delete\",        \"id\":\"X\"}
  {\"op\":\"set_attr\",      \"id\":\"X\", \"key\":\"k\", \"value\": <str|int|bool>}
  {\"op\":\"remove_attr\",   \"id\":\"X\", \"key\":\"k\"}

A {block} descriptor is:

  {
    \"kind\":   \"x-note\",
    \"id\":     \"my-id\",
    \"attrs\":  {\"desc\": \"...\"},
    \"content\":{\"type\":\"fenced|inline|items|empty\", \"value\": ...}
  }

CONTENT TYPE rules:

  inline   for h1-h4 / p / ref       value: [{\"kind\":\"text\",\"text\":\"...\"}]
  items    for ul / ol / quote       value: [[{Inline},...], ...]
  fenced   for code / raw / table /  value: \"verbatim string\"
           x-* custom blocks
  empty    for meta / include        value: omitted

GOTCHA: inline-bearing tags (h1-h4, p) take their text via inline
content, NOT a fence body. If you put your body in `content.value`
as a fenced string on a heading, the parser treats it as plain text
of the heading, not as a separate body.

EXAMPLES:

  # add a memory entry under #h-feedback heading
  agd edit memory.agd -i --op '{
    \"op\":\"insert_after\",\"id\":\"h-feedback\",
    \"block\":{\"kind\":\"x-feedback\",\"id\":\"feedback-X\",
              \"attrs\":{\"desc\":\"...\"},
              \"content\":{\"type\":\"fenced\",\"value\":\"body...\"}}}'

  # mark an entry as done by setting an attribute
  agd edit doc.agd -i --op '{\"op\":\"set_attr\",\"id\":\"task-3\",\"key\":\"done\",\"value\":true}'
")]
    Edit {
        file: PathBuf,
        /// JSON Operation. See `agd edit --help` for the schema and examples.
        #[arg(long = "op")]
        op_json: String,
        #[arg(short, long)]
        in_place: bool,
    },
    /// Verify that every `@ref #id` resolves to a real block.
    Ref {
        file: PathBuf,
        #[arg(long)]
        check: bool,
    },
    /// List every addressable block id (the document's table of contents).
    /// Useful as the entry-point for selective retrieval workflows: pull
    /// the TOC first, decide which block to fetch, then `agd get`.
    Ids {
        file: PathBuf,
        /// Emit one id per line (default) or as a JSON array.
        #[arg(long)]
        json: bool,
        /// Restrict to ids whose corresponding block has this kind.
        #[arg(long)]
        kind: Option<String>,
    },
    /// Print one or more addressable blocks by id, in canonical AGD form.
    /// Pass multiple ids in a single call to amortise the parse cost
    /// across all of them — the file is parsed once, blocks are fetched
    /// from the in-memory index. Two optional flags expand the result
    /// set in graph-theoretic ways: `--with-backlinks` appends every
    /// block that points to the requested ids; `--follow-refs` walks
    /// the `refs=` attribute (and inline `@ref`) outbound up to
    /// `--depth` hops. Both flags are cycle-safe via id deduplication.
    Get {
        file: PathBuf,
        /// One or more block ids (with or without leading `#`).
        #[arg(required = true, num_args = 1..)]
        ids: Vec<String>,
        /// Emit blocks as a JSON array of AST nodes instead of AGD bytes.
        #[arg(long)]
        json: bool,
        /// For each requested id, also include blocks that reference
        /// it (inverse of `agd backlinks`, inlined into the get).
        #[arg(long)]
        with_backlinks: bool,
        /// Follow `refs=` attribute and inline `@ref` outbound,
        /// transitively up to `--depth` hops.
        #[arg(long)]
        follow_refs: bool,
        /// Max hops for `--follow-refs`. Default 1. Ignored without
        /// `--follow-refs`.
        #[arg(long, default_value_t = 1, requires = "follow_refs")]
        depth: usize,
    },
    /// Search block bodies for a substring. Returns the matching block
    /// ids and a short excerpt around each match. Cheap entry point for
    /// "where did I write about X?" without needing to know the id.
    Search {
        file: PathBuf,
        /// Substring to search for in block bodies (Inline / Items / Fenced).
        query: String,
        /// Case-insensitive search.
        #[arg(short = 'i', long)]
        ignore_case: bool,
        /// Restrict to blocks of this kind (e.g. `x-feedback`).
        #[arg(long)]
        kind: Option<String>,
        /// Emit JSON instead of plain-text rows.
        #[arg(long)]
        json: bool,
    },
    /// List blocks that reference a given id. Inverse of `agd ref`:
    /// answers "who points to this block?". Two reference channels are
    /// recognised: inline `@ref` nodes (existing) and the `refs=`
    /// attribute convention (`refs="#a,#b,#c"` on any block, leading
    /// `#` optional). Fenced bodies are treated as opaque text.
    Backlinks {
        file: PathBuf,
        /// Target block id (with or without leading `#`).
        id: String,
        /// Emit JSON instead of plain-text rows.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
enum ConvertKind {
    /// Markdown → AGD.
    FromMd { file: PathBuf, #[arg(short, long)] out: Option<PathBuf> },
    /// AGD → Markdown.
    ToMd   { file: PathBuf, #[arg(short, long)] out: Option<PathBuf> },
    /// AGD → minimal HTML.
    ToHtml { file: PathBuf, #[arg(short, long)] out: Option<PathBuf> },
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum BenchEncoding {
    Cl100k,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("agd: {e:#}");
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    match cli.cmd {
        Cmd::Parse { file, json } => cmd_parse(&file, json),
        Cmd::Validate { file } => cmd_validate(&file),
        Cmd::Format { file, in_place, check } => cmd_format(&file, in_place, check),
        Cmd::Convert { kind } => cmd_convert(kind),
        Cmd::Bench { file } => cmd_bench(&file),
        Cmd::Id { file, add, strip, in_place } => cmd_id(&file, add, strip, in_place),
        Cmd::Edit { file, op_json, in_place } => cmd_edit(&file, &op_json, in_place),
        Cmd::Ref { file, check } => cmd_ref(&file, check),
        Cmd::Ids { file, json, kind } => cmd_ids(&file, json, kind.as_deref()),
        Cmd::Get { file, ids, json, with_backlinks, follow_refs, depth } => {
            cmd_get(&file, &ids, json, with_backlinks, follow_refs, depth)
        }
        Cmd::Search { file, query, ignore_case, kind, json } => {
            cmd_search(&file, &query, ignore_case, kind.as_deref(), json)
        }
        Cmd::Backlinks { file, id, json } => cmd_backlinks(&file, &id, json),
    }
}

fn cmd_ids(file: &Path, json: bool, kind_filter: Option<&str>) -> Result<ExitCode> {
    let src = read_input(file)?;
    let doc = parse(&src)?;
    let mut rows: Vec<(String, String, Option<String>)> = Vec::new();
    for b in &doc.blocks {
        let Some(id) = &b.id else { continue };
        if let Some(k) = kind_filter {
            if b.kind.as_str() != k {
                continue;
            }
        }
        // Surface a `desc=` attribute when present — short one-liner that
        // tells an agent whether this block is worth fetching, without
        // pulling the body. The convention is voluntary: blocks without
        // desc= just don't get one in the TOC.
        let desc = b
            .attrs
            .get("desc")
            .and_then(|v| v.as_str().map(str::to_string));
        rows.push((id.clone(), b.kind.as_str().to_string(), desc));
    }
    if json {
        let arr: Vec<serde_json::Value> = rows
            .into_iter()
            .map(|(id, kind, desc)| {
                let mut obj = serde_json::json!({"id": id, "kind": kind});
                if let Some(d) = desc {
                    obj["desc"] = serde_json::Value::String(d);
                }
                obj
            })
            .collect();
        println!("{}", serde_json::to_string(&arr)?);
    } else {
        for (id, _kind, desc) in rows {
            match desc {
                Some(d) => println!("{id}\t{d}"),
                None => println!("{id}"),
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_get(
    file: &Path,
    ids: &[String],
    json: bool,
    with_backlinks: bool,
    follow_refs: bool,
    depth: usize,
) -> Result<ExitCode> {
    use std::collections::BTreeSet;
    let src = read_input(file)?;
    let doc = parse(&src)?;

    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut order: Vec<String> = Vec::new();
    let requested: Vec<String> = ids
        .iter()
        .map(|s| s.strip_prefix('#').unwrap_or(s).to_string())
        .collect();
    for id in &requested {
        if visited.contains(id) { continue; }
        doc.find(id)
            .ok_or_else(|| anyhow!("no block with id `{id}`"))?;
        visited.insert(id.clone());
        order.push(id.clone());
    }

    if follow_refs {
        let mut frontier: Vec<String> = requested.clone();
        for _ in 0..depth {
            let mut next: Vec<String> = Vec::new();
            for fid in &frontier {
                let Some(b) = doc.find(fid) else { continue };
                let mut targets: Vec<String> = Vec::new();
                if let Some(refs_attr) = b.attrs.get("refs").and_then(|v| v.as_str()) {
                    for raw in refs_attr.split(',') {
                        let r = raw.trim().trim_start_matches('#').to_string();
                        if !r.is_empty() {
                            targets.push(r);
                        }
                    }
                }
                visit_refs(&b.content, &mut |t| targets.push(t.to_string()));
                for t in targets {
                    if visited.contains(&t) { continue; }
                    if doc.find(&t).is_none() { continue; }
                    visited.insert(t.clone());
                    order.push(t.clone());
                    next.push(t);
                }
            }
            if next.is_empty() { break; }
            frontier = next;
        }
    }

    if with_backlinks {
        for target in &requested {
            for b in &doc.blocks {
                let Some(bid) = &b.id else { continue };
                if visited.contains(bid) { continue; }
                let mut hit = false;
                visit_refs(&b.content, &mut |t| {
                    if t == target { hit = true; }
                });
                if !hit {
                    if let Some(refs_attr) = b.attrs.get("refs").and_then(|v| v.as_str()) {
                        for raw in refs_attr.split(',') {
                            let r = raw.trim().trim_start_matches('#');
                            if r == target { hit = true; break; }
                        }
                    }
                }
                if hit {
                    visited.insert(bid.clone());
                    order.push(bid.clone());
                }
            }
        }
    }

    let blocks: Vec<agd::Block> = order
        .iter()
        .filter_map(|id| doc.find(id).map(|b| b.clone()))
        .collect();

    if json {
        println!("{}", serde_json::to_string_pretty(&blocks)?);
    } else {
        let one = agd::Document::with_blocks(blocks);
        io::stdout().write_all(serialize(&one).as_bytes())?;
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_search(
    file: &Path,
    query: &str,
    ignore_case: bool,
    kind_filter: Option<&str>,
    json: bool,
) -> Result<ExitCode> {
    use agd::ast::{BlockContent, Inline};
    let src = read_input(file)?;
    let doc = parse(&src)?;
    let needle = if ignore_case { query.to_lowercase() } else { query.to_string() };

    let inline_text = |nodes: &[Inline]| -> String {
        let mut s = String::new();
        for n in nodes {
            match n {
                Inline::Text(t) | Inline::Bold(t) | Inline::Italic(t) | Inline::Code(t) | Inline::Ref(t) => s.push_str(t),
            }
        }
        s
    };
    let block_text = |b: &agd::Block| -> String {
        match &b.content {
            BlockContent::Inline(v) => inline_text(v),
            BlockContent::Items(items) => items.iter().map(|line| inline_text(line)).collect::<Vec<_>>().join("\n"),
            BlockContent::Fenced(s) => s.clone(),
            BlockContent::Empty => String::new(),
        }
    };
    let excerpt = |body: &str, hay: &str| -> String {
        if let Some(idx) = hay.find(&needle) {
            let start = idx.saturating_sub(40);
            let end = (idx + needle.len() + 40).min(body.len());
            // Find safe char boundaries for `body` (since hay may be lowercased copy of body)
            let mut s = start;
            while s > 0 && !body.is_char_boundary(s) { s -= 1; }
            let mut e = end;
            while e < body.len() && !body.is_char_boundary(e) { e += 1; }
            let slice = &body[s..e];
            let prefix = if s > 0 { "…" } else { "" };
            let suffix = if e < body.len() { "…" } else { "" };
            format!("{prefix}{}{suffix}", slice.replace('\n', " "))
        } else {
            String::new()
        }
    };

    let mut hits: Vec<serde_json::Value> = Vec::new();
    for b in &doc.blocks {
        let id = match &b.id { Some(i) => i.clone(), None => continue };
        if let Some(k) = kind_filter {
            if b.kind.as_str() != k { continue; }
        }
        let body = block_text(b);
        let hay = if ignore_case { body.to_lowercase() } else { body.clone() };
        if hay.contains(&needle) {
            hits.push(serde_json::json!({
                "id": id,
                "kind": b.kind.as_str(),
                "excerpt": excerpt(&body, &hay),
            }));
        }
    }

    if json {
        println!("{}", serde_json::to_string(&hits)?);
    } else {
        for h in &hits {
            let id = h["id"].as_str().unwrap_or("");
            let kind = h["kind"].as_str().unwrap_or("");
            let excerpt = h["excerpt"].as_str().unwrap_or("");
            println!("{id}\t{kind}\t{excerpt}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_backlinks(file: &Path, target: &str, json: bool) -> Result<ExitCode> {
    let target = target.strip_prefix('#').unwrap_or(target);
    let src = read_input(file)?;
    let doc = parse(&src)?;
    let mut rows: Vec<(String, String, Option<String>)> = Vec::new();
    for b in &doc.blocks {
        let Some(id) = &b.id else { continue };
        if id == target { continue; }
        let mut hit = false;
        visit_refs(&b.content, &mut |t| {
            if t == target { hit = true; }
        });
        // Convention: attribute `refs="#a,#b,#c"` declares outbound
        // links without polluting the body. Comma-separated, leading
        // `#` optional, whitespace tolerated.
        if !hit {
            if let Some(refs_attr) = b.attrs.get("refs").and_then(|v| v.as_str()) {
                for raw in refs_attr.split(',') {
                    let r = raw.trim().trim_start_matches('#');
                    if r == target {
                        hit = true;
                        break;
                    }
                }
            }
        }
        if !hit { continue; }
        let desc = b
            .attrs
            .get("desc")
            .and_then(|v| v.as_str().map(str::to_string));
        rows.push((id.clone(), b.kind.as_str().to_string(), desc));
    }
    if json {
        let arr: Vec<serde_json::Value> = rows
            .into_iter()
            .map(|(id, kind, desc)| {
                let mut obj = serde_json::json!({"id": id, "kind": kind});
                if let Some(d) = desc {
                    obj["desc"] = serde_json::Value::String(d);
                }
                obj
            })
            .collect();
        println!("{}", serde_json::to_string(&arr)?);
    } else {
        for (id, kind, desc) in rows {
            match desc {
                Some(d) => println!("{id}\t{kind}\t{d}"),
                None => println!("{id}\t{kind}"),
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_parse(file: &Path, json: bool) -> Result<ExitCode> {
    let src = read_input(file)?;
    let doc = parse(&src).with_context(|| format!("parsing {}", file.display()))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&doc)?);
    } else {
        println!("{:#?}", doc);
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_validate(file: &Path) -> Result<ExitCode> {
    let src = read_input(file)?;
    match parse(&src) {
        Ok(_) => {
            eprintln!("{}: OK", file.display());
            Ok(ExitCode::SUCCESS)
        }
        Err(e) => {
            eprintln!("{}: {e}", file.display());
            Ok(ExitCode::from(1))
        }
    }
}

fn cmd_format(file: &Path, in_place: bool, check: bool) -> Result<ExitCode> {
    let src = read_input(file)?;
    let doc = parse(&src)?;
    let out = serialize(&doc);
    if check {
        if out == src {
            return Ok(ExitCode::SUCCESS);
        }
        eprintln!("{}: not canonical", file.display());
        return Ok(ExitCode::from(1));
    }
    if in_place {
        write_in_place(file, &doc, &out)?;
    } else {
        io::stdout().write_all(out.as_bytes())?;
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_convert(kind: ConvertKind) -> Result<ExitCode> {
    match kind {
        ConvertKind::FromMd { file, out } => {
            let src = fs::read_to_string(&file)?;
            let agd = from_markdown(&src);
            emit(out.as_deref(), &agd)
        }
        ConvertKind::ToMd { file, out } => {
            let src = fs::read_to_string(&file)?;
            let doc = parse(&src)?;
            let md = to_markdown(&doc);
            emit(out.as_deref(), &md)
        }
        ConvertKind::ToHtml { file, out } => {
            let src = fs::read_to_string(&file)?;
            let doc = parse(&src)?;
            let html = to_html(&doc);
            emit(out.as_deref(), &html)
        }
    }
}

fn cmd_bench(file: &Path) -> Result<ExitCode> {
    let src = fs::read_to_string(file)?;
    let doc = parse(&src)?;
    let agd_canon = serialize(&doc);
    let md = to_markdown(&doc);
    let html = to_html(&doc);
    let json = serde_json::to_string(&doc)?;
    let bpe = tiktoken_rs::cl100k_base().map_err(|e| anyhow!("{e}"))?;
    let agd_n = bpe.encode_with_special_tokens(&agd_canon).len();
    let md_n = bpe.encode_with_special_tokens(&md).len();
    let html_n = bpe.encode_with_special_tokens(&html).len();
    let json_n = bpe.encode_with_special_tokens(&json).len();
    println!("file:     {}", file.display());
    println!("encoding: cl100k_base");
    println!("AGD:      {agd_n} tokens");
    println!("Markdown: {md_n} tokens  ({:+.1}% vs AGD)", pct(md_n, agd_n));
    println!("HTML:     {html_n} tokens  ({:+.1}% vs AGD)", pct(html_n, agd_n));
    println!("JSON:     {json_n} tokens  ({:+.1}% vs AGD)", pct(json_n, agd_n));
    Ok(ExitCode::SUCCESS)
}

fn cmd_id(file: &Path, add: bool, strip: bool, in_place: bool) -> Result<ExitCode> {
    if !add && !strip {
        bail!("specify --add or --strip");
    }
    if add && strip {
        bail!("cannot combine --add and --strip");
    }
    let src = read_input(file)?;
    let mut doc = parse(&src)?;
    if add {
        id::auto_assign(&mut doc);
    }
    if strip {
        id::strip_all(&mut doc);
    }
    let out = serialize(&doc);
    if in_place {
        write_in_place(file, &doc, &out)?;
    } else {
        io::stdout().write_all(out.as_bytes())?;
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_edit(file: &Path, op_json: &str, in_place: bool) -> Result<ExitCode> {
    let src = read_input(file)?;
    let mut doc = parse(&src)?;
    let op: Operation = serde_json::from_str(op_json)
        .with_context(|| format!("parsing operation JSON: {op_json}"))?;
    doc.apply(op)?;
    let out = serialize(&doc);
    if in_place {
        write_in_place(file, &doc, &out)?;
    } else {
        io::stdout().write_all(out.as_bytes())?;
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_ref(file: &Path, check: bool) -> Result<ExitCode> {
    let src = read_input(file)?;
    let doc = parse(&src)?;
    if check {
        match check_refs(&doc) {
            Ok(()) => {
                eprintln!("{}: refs OK", file.display());
                Ok(ExitCode::SUCCESS)
            }
            Err(e) => {
                eprintln!("{}: {e}", file.display());
                Ok(ExitCode::from(1))
            }
        }
    } else {
        // List all refs
        for block in &doc.blocks {
            visit_refs(&block.content, &mut |target| {
                println!("{}", target);
            });
        }
        Ok(ExitCode::SUCCESS)
    }
}

fn visit_refs(content: &agd::BlockContent, f: &mut impl FnMut(&str)) {
    match content {
        agd::BlockContent::Inline(v) => {
            for n in v {
                if let agd::Inline::Ref(t) = n {
                    f(t);
                }
            }
        }
        agd::BlockContent::Items(items) => {
            for it in items {
                for n in it {
                    if let agd::Inline::Ref(t) = n {
                        f(t);
                    }
                }
            }
        }
        _ => {}
    }
}

fn read_input(file: &Path) -> Result<String> {
    if file.as_os_str() == "-" {
        let mut s = String::new();
        io::stdin().read_to_string(&mut s)?;
        Ok(s)
    } else {
        fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))
    }
}

fn ensure_real_path(file: &Path) -> Result<()> {
    if file.as_os_str() == "-" {
        bail!("--in-place not supported with stdin");
    }
    Ok(())
}

// Last line of defence before overwriting a file in place. `apply()` and the
// edit validators are the primary guards, but they cannot know about every
// present-or-future gap in the serializer. Here we prove the round-trip: the
// bytes we are about to write must re-parse, and re-parse back to the SAME
// document. That catches both failure modes at once — output no later parse can
// read (the exact bug that corrupted a real memory file) AND output that parses
// to a *different* document (unrepresentable content silently rewritten, e.g. a
// delimiter or reference the v0.1 syntax cannot encode). `Block` equality
// ignores source spans, so canonical reformatting never trips it. Cheap: the
// write already dominates the cost, and any failure leaves the file untouched.
fn write_in_place(file: &Path, doc: &Document, out: &str) -> Result<()> {
    ensure_real_path(file)?;
    let reparsed = parse(out).with_context(|| {
        format!(
            "refusing to write {}: serialized output does not re-parse — this \
             is a bug in agd; the file was left untouched",
            file.display()
        )
    })?;
    if &reparsed != doc {
        bail!(
            "refusing to write {}: this edit does not round-trip — the \
             serialized output re-parses to a different document (content the \
             v0.1 syntax cannot represent); the file was left untouched",
            file.display()
        );
    }
    fs::write(file, out)?;
    Ok(())
}

fn emit(out: Option<&Path>, content: &str) -> Result<ExitCode> {
    match out {
        Some(p) => fs::write(p, content).with_context(|| format!("writing {}", p.display()))?,
        None => io::stdout().write_all(content.as_bytes())?,
    }
    Ok(ExitCode::SUCCESS)
}

fn pct(n: usize, base: usize) -> f64 {
    if base == 0 { return 0.0; }
    (n as f64 - base as f64) / (base as f64) * 100.0
}

fn _unused() {
    // referenced to silence unused import warnings during partial builds
    let _: Document = Document::default();
}
