# sift — Structural codebase index for LLM tooling

[![Crates.io](https://img.shields.io/crates/v/code-sift.svg)](https://crates.io/crates/code-sift)
[![CI](https://github.com/rsarv3006/sift/actions/workflows/ci.yml/badge.svg)](https://github.com/rsarv3006/sift/actions/workflows/ci.yml)
[![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

`sift` builds a language-agnostic structural index of a codebase using tree-sitter
and optionally enriches it with semantic embeddings (candle or API-based) for
natural-language code search. It is meant to be used as an LLM CLI skill: the LLM
calls `sift query` to find definitions, trace calls, and explore code relationships
— without needing embeddings or API calls for structural queries, and with
embeddings for semantic ones.

## Status

Structural index is stable. Semantic embedding layer is new.

- Rust crate compiles and passes clippy (cognitive complexity ≤ 15)
- Indexes Rust, Python, JavaScript, TypeScript, TSX, Go, C, C++, Java, Ruby, Zig, Bash via tree-sitter
- Captures: function/struct/trait/enum/class/type/interface definitions, call sites (method calls, qualified calls), and import/include statements
- Cross-file import resolution — imports link to the defining symbol's file/line/kind
- Query commands: `define`, `calls`, `callees`, `implements`, `imports`, `importers`, `file`, `files`, `symbols matching`, `semantic`
- `sift skill` outputs a ready-to-use LLM tool definition (OpenAI-compatible)
- Semantic embeddings: API-based (OpenAI-compatible) always available; local inference via [candle](https://github.com/huggingface/candle) with the `candle` feature
- Agentic benchmark harness in `bench-fixtures/` — 30/30 structural tasks pass (17x avg token savings), including pagination benchmarks; 20/20 embedding tasks pass (requires API embedder)
- Incremental re-index: file mtimes tracked on each index, subsequent runs only re-parse changed files. Re-index is O(changed) not O(total)
- Auto-re-index on stale `sift query`: transparently rebuilds the index when source files change
- `sift watch` daemon: uses `notify` 7 to monitor filesystem and re-index automatically on every change (non-blocking thread, 500ms debounce)
- Atomic index save: `.tmp` + `rename` prevents partial-read races; V2 magic prefix for backward compat
- Unit tests covering parser, index, query, event filtering, and mtime collection
- No functions excluded from the complexity threshold

## Install

```bash
cargo install code-sift                        # from crates.io
cargo install --features candle code-sift      # with local embeddings
```

Or build from source:
```bash
git clone https://github.com/rsarv3006/sift
cd sift
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
  JavaScript, TypeScript, TSX, Go, C, C++, Java, Ruby, Zig, and Bash. No network required.
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

# Watch for changes and auto-re-index
sift watch                              # watches current directory
sift watch /path/to/project --embed     # + semantic embeddings on re-index

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

## Embedding Configuration

Semantic search is optional. When you pass `--embed`, sift checks these
sources (later wins):

1. Hardcoded defaults
2. `~/.config/sift/config.toml` (user-level)
3. `.sift/config.toml` (project-level, relative to cwd)
4. `SIFT_EMBED_*` environment variables

Example project config (`.sift/config.toml`):

```toml
[embed]
backend = "api"
api_url = "http://10.0.0.39:11434/v1/embeddings"
api_model = "nomic-embed-text"
```

Once set, commands work without env vars:

```bash
sift index --embed .                         # reads config
sift query --embed "semantic handle http request"
```

Env vars override config files and are useful for per-invocation overrides:

| Variable | Default | Description |
|----------|---------|-------------|
| `SIFT_EMBED_BACKEND` | `auto` | `api`, `local` (requires `candle` feature), or `auto` |
| `SIFT_EMBED_API_KEY` | — | API key (not needed for Ollama/local endpoints) |
| `SIFT_EMBED_API_URL` | `https://api.openai.com/v1/embeddings` | API endpoint |
| `SIFT_EMBED_API_MODEL` | `text-embedding-3-small` | Model name for API backend |
| `SIFT_EMBED_MODEL_PATH` | — | Path to local model files (`candle` feature only) |
| `OPENAI_API_KEY` | — | Fallback if `SIFT_EMBED_API_KEY` is unset |

If no embedding backend is available, `sift` prints a warning at index time
telling you what to set.

Build with candle for fully local embeddings:
```bash
cargo build --release --features candle
sift index --embed .                         # uses candle automatically
```

## Checking

```bash
make check       # lint + test + complexity
make lint        # cargo clippy (cognitive complexity threshold: 15)
make test        # unit tests (parser, index, query)
make complexity  # arborist-cli cyclomatic/cognitive complexity
make bench              # synthetic codebase benchmark (25 correctness tasks)
make bench-embed        # embedding benchmark (20 semantic tasks, requires API embedder)
make bench-incremental  # incremental re-index benchmark (time savings vs full)
make bench-real         # real-repo benchmark (requires cloned repo in /tmp/just)
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
- [x] Unit tests across parser, index, query, event filtering, and mtime collection
- [x] Cyclomatic/cognitive complexity checking (clippy + arborist, threshold 15)
- [x] Semantic embedding layer — candle (local) + API fallback, computed during `sift index --embed`, queried via `sift query --embed "semantic ..."`
- [x] Language support: 12 languages via tree-sitter (Rust, Python, JS/TS/TSX, Go, C, C++, Java, Ruby, Zig, Bash)
- [x] Cross-file import resolution — each import links to the defining symbol's file/line/kind
- [x] Agentic benchmark harness (`make bench`) — 30 correctness tasks across 2 synthetic codebases, including pagination verification
- [x] Embedding benchmark harness (`make bench-embed`) — 20 semantic search tasks, requires API embedder
- [x] API key optional for embedding (works with Ollama, local LLMs, etc.)
- [x] Doc comment extraction — captures `///`, `/** */`, `#`, `//` doc comments preceding definitions; included in JSON output and embedding text for better semantic search
- [x] Binary index format (`bincode`) — serializes to `.sift/index.bin`, faster load/save than JSON for large codebases
- [x] Incremental re-index — file mtimes tracked, re-parses only changed files
- [x] Auto-re-index on stale query — transparent rebuild when source files change
- [x] `sift watch` daemon — filesystem watcher for continuous auto-re-index
- [x] `implements` by trait name — `implements <name>` now finds impl blocks by both type name and trait name (second tree-sitter pattern captures `trait: (type_identifier)` on `impl_item`)
- [x] `sift query` pagination — `--limit` and `--offset` flags for paginating large result sets
- [x] Per-language indexing performance — `sift index` now prints per-language parse speed (files, avg ms/file, total CPU time) to help identify slow grammars

### Next

- **Better semantic embeddings for code** — currently embeds symbol name + kind + doc text; consider code-specific models (e.g. starcoder2) or including function signature for richer context.
