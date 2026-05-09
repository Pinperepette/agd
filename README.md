# AGD — Agent Document Format

A line-oriented text format optimised for LLM agents. Sits between Markdown
(too ambiguous for safe machine editing) and HTML/XML (too verbose for
token-constrained contexts). Every block is independently addressable via
a stable `[#id]`, so multiple agents can edit the same document without
fighting over byte ranges.

```agd
@meta title="Welcome" author=alice

@h1 Hello [#intro]
@p AGD blocks start with `@<tag>` at column 0 — that is the entire syntactic story.

@h2 Features [#features]
@ul
- token-efficient vs HTML and JSON
- deterministic LL(1) parsing
- stable per-block IDs for multi-agent editing
- canonical form: byte-stable round-trip

@code lang=python [#hello]
~~~
def hello():
    return "world"
~~~

@ref #hello
```

## Why a new format?

Markdown is great for humans. But ambiguity (CommonMark vs GFM vs MDX),
implicit semantics (paragraph boundaries, nested-list rules), and lack
of stable per-block addresses make it painful for LLM agents that need
to edit, re-edit, and merge changes deterministically. HTML and XML
solve those problems but cost 25–60% more tokens. JSON is unambiguous
but illegible in `cat`.

AGD trades a small token premium against Markdown for a *radical*
simplification of the parser, plus first-class block IDs that turn
edit operations into one-line JSON instead of fragile text diffs.

## Install

```sh
cargo install --path .
```

The binary is called `agd`.

## Quickstart

```sh
agd validate          examples/api-doc.agd
agd parse --json      examples/api-doc.agd
agd format --check    examples/api-doc.agd
agd convert from-md   README.md
agd convert to-md     spec/AGD-SPEC.agd
agd convert to-html   examples/tutorial.agd
agd bench             examples/api-doc.agd
agd id   --add        my-doc.agd
agd ref  --check      my-doc.agd
```

Editing a single block by ID:

```sh
agd edit doc.agd --op '{
  "op":   "set_attr",
  "id":   "intro",
  "key":  "lang",
  "value":"english"
}'
```

## Library API

```rust
use agd::{parse, serialize, edit::Operation};

let mut doc = parse("@h1 Hello [#intro]\n@p Body\n")?;
doc.apply(Operation::SetAttr {
    id:    "intro".into(),
    key:   "lang".into(),
    value: "english".into(),
})?;
println!("{}", serialize(&doc));
```

See `src/edit.rs` for the full operation algebra.

## Token economy

Measured with `cl100k_base` over `examples/`. See `benches/RESULTS.md`
for the live numbers.

| Format       | vs AGD          |
|--------------|-----------------|
| HTML         | **+10 to +30%** more tokens |
| JSON         | **+115 to +200%** more tokens |
| Markdown     | -15 to -25% — Markdown wins on raw count |

Markdown is consistently smaller — that is the honest tradeoff. AGD
costs ~20% more tokens than CommonMark in exchange for unambiguous
parsing and stable block IDs. If you only need to render prose to a
human, use Markdown. If an agent has to *edit* the document, AGD pays
back fast.

## Specification

The format is specified in `spec/AGD-SPEC.agd` (and rendered to
`spec/AGD-SPEC.md` by the CLI). The grammar lives in
`grammar/agd.ebnf`.

## Project layout

```
agd/
├── Cargo.toml
├── grammar/agd.ebnf            frozen v0.1 grammar
├── spec/                       spec source (.agd) + rendered (.md)
├── examples/                   four real-world documents
├── benches/                    token + parse benchmarks
├── src/
│   ├── lib.rs                  public API
│   ├── lexer.rs                line classifier (~150 lines)
│   ├── parser.rs               block assembler + inline parser
│   ├── ast.rs                  type system
│   ├── serializer.rs           canonical form
│   ├── edit.rs                 operation algebra
│   ├── id.rs                   ID slugging + content hashing
│   ├── convert/                MD ↔ AGD ↔ HTML
│   └── bin/agd.rs              CLI driver
└── tests/
    ├── conformance/            paired .agd / .json fixtures
    ├── conformance.rs          corpus runner
    ├── roundtrip.rs            proptest: serialize → parse → equal
    └── cli.rs                  binary integration tests
```

## Status

v0.1 — grammar frozen, full toolchain, ≥ 67 tests across unit /
property / conformance / CLI suites.

## License

MIT.
