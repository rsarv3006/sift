# sift — Structural codebase index for LLM tooling

[![Crates.io](https://img.shields.io/crates/v/code-sift.svg)](https://crates.io/crates/code-sift)
[![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

`sift` builds a language-agnostic structural index of a codebase using tree-sitter
and optionally enriches it with semantic embeddings (candle or API-based) for
natural-language code search. It is meant to be used as an LLM CLI skill: the LLM
calls `sift query` to find definitions, trace calls, and explore code relationships
— without needing embeddings or API calls for structural queries, and with
embeddings for semantic ones.

## Status

Structural index is stable. Semantic embedding layer is new.

- Rust crate compiles, passes clippy (cognitive complexity ≤ 15), and 65 unit tests
- Indexes Rust, Python, JavaScript, TypeScript, TSX, Go, C, C++, Java, Ruby, Zig via tree-sitter
- Captures: function/struct/trait/enum/class/type/interface definitions, call sites (method calls, qualified calls), and import/include statements
- Cross-file import resolution — imports link to the defining symbol's file/line/kind
- Query commands: `define`, `calls`, `callees`, `implements`, `imports`, `importers`, `file`, `files`, `symbols matching`, `semantic`
- `sift skill` outputs a ready-to-use LLM tool definition (OpenAI-compatible)
- Semantic embeddings: API-based (OpenAI-compatible) always available; local inference via [candle](https://github.com/huggingface/candle) with the `candle` feature
- Agentic benchmark harness in `bench-fixtures/` — 25/25 structural tasks pass (17x avg token savings), 20/20 embedding tasks pass (requires API embedder)
- No functions excluded from the complexity threshold

## Install

```bash
cargo install code-sift                        # from crates.io
cargo install --features candle code-sift      # with local embeddings
```

Or build from source:
```bash
git clone https://github.com/anomalyco/coder-i-ardly-knew-er
cd coder-i-ardly-knew-er
cargo build --release
./target/release/sift --help
```

## Philosophy

- **Zero-token structural queries**: Most code understanding tasks (find definition,
  trace callers, list symbols in a file) are purely structural and need zero LLM
  tokens when served by sift.
- **LLM skill first**: sift is designed to be invoked by an LLM as a tool.
  `sift skill` outputs the tool definition for plugging into an LLM system prompt.
- **Local by default, language-agnostic**: tree-sitter parsers for Rust, Python,
  JavaScript, TypeScript, TSX, Go, C, C++, Java, Ruby, and Zig. No network required.
- **Optional semantic search**: compute embeddings during indexing (`--embed`,
  `SIFT_EMBED_*` env vars) and query with `sift query semantic ...`.

## Usage

```
# Index a codebase
sift index /path/to/project
sift index /path/to/project --embed           # + semantic embeddings

# Queries (returns JSON)
sift query "define parse_file"          # Find a definition
sift query "calls parse_file"           # Who calls it
sift query "callees parse_file"         # What it calls
sift query "implements Iterator"        # Implementations
sift query "symbols matching revenue"   # Substring name search
sift query "file main.rs"              # Symbols in a file
sift query "files"                     # All indexed files
sift query "parse_file"                # Bare name -> define
sift query --embed "semantic calculate revenue"  # Semantic search

# LLM tool definition
sift skill
```

## Output example

```json
{"type":"definition","name":"parse_file","kind":"function",
 "file":"src/parser.rs","line":154,"end_line":239}

{"type":"definition","name":"Calculator","kind":"struct",
 "file":"src/lib.rs","line":7,"end_line":9,
 "doc":"/// A calculator that chains operations and evaluates them sequentially."}
```

Semantic results include a `score` field (cosine similarity). Results with doc comments include a `doc` field:
```json
{"type":"semantic","name":"calculate_revenue","kind":"function",
 "file":"src/finance.rs","line":42,"end_line":56,"score":0.87,
 "doc":"/// Calculate monthly recurring revenue from the subscriptions list."}
```

## Building

```bash
cargo build --release                       # structural only
cargo build --release --features candle     # + local embeddings
```

Requires a C compiler (for tree-sitter grammar compilation) and Rust 1.75+.

## Checking

```bash
make check       # lint + test + complexity
make lint        # cargo clippy (cognitive complexity threshold: 15)
make test        # unit tests (parser, index, query)
make complexity  # arborist-cli cyclomatic/cognitive complexity
make bench       # synthetic codebase benchmark (25 correctness tasks)
make bench-embed # embedding benchmark (20 semantic tasks, requires API embedder)
make bench-real  # real-repo benchmark (requires cloned repo in /tmp/just)
```

The synthetic benchmark (`make bench`) indexes fixtures in `bench-fixtures/` and
verifies correctness against known-answer tasks. The real-repo benchmark
(`make bench-real`) measures token savings against an actual open-source project
(current results: **404x avg savings** over naive grep+cat on the `just` crate,
123 source files, 577KB). The embedding benchmark (`make bench-embed`) tests
semantic search correctness and token savings using an API embedder (e.g. Ollama
with `nomic-embed-text`). Configure via `SIFT_EMBED_BACKEND=api`
`SIFT_EMBED_API_URL=http://localhost:11434/v1/embeddings`.

Clippy config is in `clippy.toml`. Complexity analysis requires `arborist-cli`:
```bash
cargo install arborist-cli
```

## Architecture

```
sift index  →  tree-sitter parses files  →  extracts symbols, calls, imports
                   └─ --embed  →  candle/API  →  computes symbol embeddings
                                           →  serializes to bincode index (.sift/index.bin)
sift query  →  loads index  →  structural or semantic queries  →  JSON output
                   └─ --embed  →  candle/API  →  embeds query for semantic search
sift skill  →  prints LLM tool definition
```

## Roadmap

### Completed

- [x] Structural index (tree-sitter: symbols, calls, imports)
- [x] CLI commands: `index`, `query`, `skill`
- [x] Language support: Rust, Python, JS/TS, Go, C, C++, Java, Ruby, Zig
- [x] Import and method call capture
- [x] Caller name resolution (span-based)
- [x] 65 unit tests across parser, index, and query modules
- [x] Cyclomatic/cognitive complexity checking (clippy + arborist, threshold 15)
- [x] Semantic embedding layer — candle (local) + API fallback, computed during `sift index --embed`, queried via `sift query --embed "semantic ..."`
- [x] Language support: 11 languages via tree-sitter
- [x] Cross-file import resolution — each import links to the defining symbol's file/line/kind
- [x] Agentic benchmark harness (`make bench`) — 25 correctness tasks across 2 synthetic codebases
- [x] Embedding benchmark harness (`make bench-embed`) — 20 semantic search tasks, requires API embedder
- [x] API key optional for embedding (works with Ollama, local LLMs, etc.)
- [x] Doc comment extraction — captures `///`, `/** */`, `#`, `//` doc comments preceding definitions; included in JSON output and embedding text for better semantic search
- [x] Binary index format (`bincode`) — serializes to `.sift/index.bin`, faster load/save than JSON for large codebases

### Next

- **`implements` by trait name** — currently `implements <name>` finds impl blocks by type name, not trait name. Need a second impl pattern that captures the trait being implemented.
- **`sift query` streaming** — for very large result sets, support pagination or streaming JSON output.
- **Per-language indexing performance** — measure parse rates per language on large real-world repos, identify slow grammars.
- **Better semantic embeddings for code** — currently embeds symbol name + kind + doc text; consider code-specific models (e.g. starcoder2) or including function signature for richer context.
