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
    /// Apply an edit operation supplied as a JSON object.
    Edit {
        file: PathBuf,
        /// JSON `{"op":"replace","id":"x","with":{...}}` etc.
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
    }
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
        ensure_real_path(file)?;
        fs::write(file, &out)?;
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
        ensure_real_path(file)?;
        fs::write(file, &out)?;
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
        ensure_real_path(file)?;
        fs::write(file, &out)?;
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
