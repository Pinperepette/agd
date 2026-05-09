<!-- license=MIT status=draft title=AGD — Agent Document Format version=0.1 -->

# AGD — Agent Document Format

Specification version 0.1. AGD is a line-oriented text format optimised for

LLM agents. It sits between Markdown and HTML: more deterministic than

Markdown, more compact than HTML, and addressable per block so multiple

agents can edit the same document without stepping on each other.

## Goals

- lower token cost than HTML or XML for the same logical content
- deterministic parsing — single-pass, LL(1), no backtracking
- human-readable in `cat file.agd`
- stable per-block IDs for safe multi-agent editing
- minimal syntax noise — one sigil, no closing tags
- convertible to and from Markdown without information loss for the common subset

## Non-goals

- displacing Markdown for human-authored prose
- matching HTML for rich visual layout
- supporting tables, footnotes, or math in v0.1

## Encoding

- UTF-8
- LF line endings (CRLF must be normalised before parsing)
- no byte-order mark
- file extension `.agd`
- MIME type `text/agd`
- magic detection: first line matches `^@meta\b`

## Lexical structure

Every line is classified independently by leading bytes.

- block-start — line starts with `@` followed by a tag-name char
- list item — line starts with `- ` (only meaningful inside `@ul` / `@ol`)
- quote line — line starts with `> ` (only meaningful inside `@quote`)
- fence — line is exactly `~~~`
- comment — line starts with `@!`
- empty — blank line; terminates open multi-line block scopes
- continuation — any other line; verbatim content of an enclosing fence

## Block tags

Built-in tags. Custom tags must be `x-` prefixed.

- `@meta` — document metadata; attributes only, no body
- `@h1` `@h2` `@h3` `@h4` — headings; inline body, optional ID
- `@p` — paragraph; inline body, optional ID
- `@ul` `@ol` — list; items follow as `- ` lines
- `@quote` — block quote; lines follow as `> ` lines
- `@code` — fenced verbatim with `lang=` attribute
- `@raw` — fenced verbatim, no language semantics
- `@table` — fenced verbatim, treated as `@code lang=csv` in v0.1
- `@ref` — block-level cross-reference; takes `#<id>` payload
- `@include` — attribute-only directive; resolution is host-defined
- `@!` — comment; preserved in AST but excluded from rendered output
- `@x-…` — custom extension blocks; semantics defined by host

## Block-start syntax

A block-start line has the form

```agd
@<tag>[ <key>=<value> …][ <inline body>][ [#<id>]]
```

with these constraints

- the `@` must be at column 0
- tag names are lowercase ASCII; custom tags start with `x-`
- attributes appear after the tag, separated by single spaces
- attribute values are bareword, signed integer, `true` / `false`, or `"quoted string"`
- only inline-bearing tags (`@h1`-`@h4`, `@p`) take a free-form inline body
- the optional `[#<id>]` is always last on the line

## Inline grammar

Within an inline body, AGD recognises three non-nestable runs and

otherwise treats every byte as plain text.

- `*bold*` — emitted as `<strong>` in HTML
- `_italic_` — emitted as `<em>` in HTML
- `` `code` `` — emitted as inline `<code>`

Mismatched delimiters degrade to plain text. Backslash escapes are not

supported. Inline references (`@ref` inside running text) are not part

of v0.1 — use a block-level `@ref #id` instead.

## Identifiers

- pattern: `[a-zA-Z_][a-zA-Z0-9_-]*`
- unique per document — duplicate IDs raise a parse error
- optional content-hash suffix `[#name:abcd1234]` — 8-hex SHA-1 prefix; parsed in v0.1, validation deferred to v0.2

## Fenced content

`@code`, `@raw`, `@table`, and `@x-*` blocks may carry a verbatim body

between two `~~~` fences. The block-start line carries any attributes

(e.g. `lang=rust`); the body sits between the opening and closing

fence lines and is preserved byte-for-byte (modulo the single LF the

parser strips before the closing fence).

A line equal to `~~~` always closes the current fence — there is no

fence-depth escape in v0.1. Content that needs to embed `~~~` must use

`@raw` and a different sentinel chosen at the application layer.

## Canonical form

Two AGD documents are canonically equivalent when, after parsing, their

ASTs are structurally equal. The canonical serialiser produces:

- LF line endings
- single space between tokens on the block-start line
- attributes sorted alphabetically by key
- ID always last on the block-start line
- one blank line between top-level blocks
- no trailing whitespace on any line
- bareword values where possible; quoted strings only when necessary

## Parser shape

A reference parser fits in roughly 300 lines of any modern language.

1. lexer — line classifier producing a stream of typed lines
2. block assembler — three-state machine consuming the line stream
3. inline parser — left-to-right scan over inline-bearing block bodies
4. AST validator — ID uniqueness and (optionally) reference resolution

## Editing model

Agents edit AGD documents through a small operation algebra. Operations

are pure data — JSON-serialisable, replayable, conflict-detectable.

```json
{"op": "replace",       "id": "intro", "with": {…block…}}
{"op": "insert_after",  "id": "intro", "block": {…}}
{"op": "insert_before", "id": "intro", "block": {…}}
{"op": "delete",        "id": "intro"}
{"op": "set_attr",      "id": "intro", "key": "lang", "value": "rust"}
{"op": "remove_attr",   "id": "intro", "key": "lang"}
```

Two agents working on the same document apply ops independently. The host

resolves conflicts (last-write-wins by default). Optional content hashes

give an extra integrity check before applying.

## Token economy

Measured with `cl100k_base` on the four files in `examples/`.

```
file,                        AGD,  Markdown, HTML, JSON
api-doc.agd,                 555,   462,      701,  1465
multi-agent-task.agd,        472,   395,      525,  1017
readme.agd,                  304,   245,      404,   922
tutorial.agd,                526,   436,      649,  1531
```

Honest summary

- AGD is consistently smaller than HTML (-25% to -33%) and JSON (-50% to -67%)
- AGD is **larger** than Markdown by 15-20% — the cost of explicit `@<tag>` prefixes and `[#id]` anchors
- the tradeoff buys deterministic parsing, stable addressing, and a 200-line reference parser

## Comparison

> Pick AGD when an LLM has to **edit** the document.
> Pick Markdown when a human has to **write** it.
> Pick HTML when a browser has to **render** it.
> Pick JSON when a program has to **parse** it without ambiguity at any cost.

## Conformance

A conformant parser MUST

1. accept every input that matches the grammar in `grammar/agd.ebnf`
2. reject every input that violates an explicit invariant (duplicate ID, unterminated fence, unknown non-`x-` tag)
3. preserve all attribute keys and values verbatim
4. expose byte-offset spans for every block (recommended, not required)

## Future work

- v0.2 — content-hash validation, tree-sitter grammar, VS Code extension
- v0.3 — LSP server, fence-depth escapes, native table syntax
- v1.0 — frozen grammar, IANA MIME registration, multi-language parser ports

<!-- End of specification v0.1. -->
