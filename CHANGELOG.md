# Changelog

## [0.5.0] — 2026-05-24

### Added
- `sift query` pagination — `--limit` and `--offset` flags for paginating large result
  sets. Applied as `.skip(offset).take(limit)` on query results. Shows `showing results
  X-Y of Z` on stderr when results exceed slice size.
- `implements <name>` now matches impl blocks by trait name as well as type name —
  second tree-sitter pattern captures `(impl_item trait: (type_identifier) @name)`.
- Per-language indexing performance — `sift index` prints per-language parse speed
  (files, avg ms/file, total CPU time) to help identify slow grammars.
- `parse_duration` field on `ParsedFile` — populated by `parse_file()` via
  `Instant::now()`, used by the per-language speed reporter.
- Pagination unit tests — 7 tests in `query.rs::paginate()` covering limit, offset,
  offset+limit, offset-beyond-total, limit=0, limit-exceeds-total, disjoint-pages.
- Pagination benchmark tasks — 4 tasks across `micro-calc` (callees evaluate page 1/2)
  and `meso-server` (implements Handler page 1/2) — 30/30 structural tasks total.
- `test_parse_file_sets_parse_duration` — verifies `parse_file()` sets
  `parse_duration` to `Some(Duration > 0)`.
- `test_parse_impl_trait` — verifies `impl Operation for AddOp` captures both
  `Operation` and `AddOp`.
- Generic `benches/real-repo.sh` — language-agnostic, uses `find` with all 19 sift
  extensions; `QUERIES=` env var for override; handles "No results" gracefully.
- `make bench-real-linux` target — runs real-repo benchmark against
  `$(HOME)/dev/THIRDPARTY/linux`.

### Changed
- `LanguageId` derives `Hash` for `HashMap`-based per-language statistics.
- `parse_file()` now wraps `parse_source()` with timing instead of calling it inline.
- Bench fixture task count: 26 → 30 (4 new pagination tasks).
- `benches/real-repo.sh` defaults to release build.
- `README.md` — moved completed features (`implements` by trait, pagination,
  per-language speed) from "Next" to "Done".

### Fixed
- Clippy `clone_on_copy` warning in `src/index.rs:1270` (`make_event(kind.clone())`
  → `make_event(*kind)`).

## [0.4.0] — 2026-05-21

### Added
- Incremental re-index — `CodeIndex.file_mtimes` tracks file modification times; only
  re-parses changed files on subsequent index runs. New methods: `collect_mtimes`,
  `classify_files`, `reconstruct_parsed_file`, `preserve_embeddings`.
- Auto-re-index on stale `sift query` — transparently rebuilds the index when source
  files have changed, with timing logged to stderr.
- `sift watch` daemon — uses `notify` 7 with `RecommendedWatcher` and recursive
  watching. 500ms debounce batching. Filters out `.sift/` paths, non-source files,
  and Access/Other event kinds.
- Non-blocking re-index in watch loop — re-index runs in a background thread so file
  events are still collected during re-building.
- `sift watch --daemonize` — fork to background with PID written to `.sift/watcher.pid`.
- `CodeIndex::apply_changes()` — incremental index update that only touches changed
  files instead of rebuilding from scratch. `reindex_incremental` now uses this,
  making re-index O(changed) instead of O(total). For a 100-file project with 1
  changed file: **0.44s vs 9.0s (20x faster)**.
- `CodeIndex::compute_missing_embeddings()` — computes embeddings only for symbols
  that don't already have one, avoiding redundant API calls.
- Progress indicator for `sift index` — prints elapsed time every 3 seconds during
  the parallel parse phase (visible on large codebases like the Linux kernel).

### Fixed
- `sift watch` no longer blocks the event loop during re-index (was blocking on
  `collect_mtimes` which walks the full project tree).
- `bench_embed::count_embedded` now uses `CodeIndex::load()` instead of raw
  `bincode::deserialize()` — fixes V2 magic prefix incompatibility.

## [0.3.0] — 2026-05-20

### Added
- Config file support — `~/.config/sift/config.toml` and `.sift/config.toml` for embedding
  settings. Three-layer merge: defaults ← user config ← project config ← env vars.
  Each layer only overrides fields that are explicitly set.
- `CHANGELOG.md` (this file).

### Fixed
- Candle 0.8 `to_vec1()` error in `embed_texts` — squeeze batch dim before converting.
- Env var check in `cmd_index` uses `config.api_url` instead of raw `std::env::var`
  so config-file URL is properly detected.

## [0.2.0] — 2026-05-20

### Added
- Bash language support — `function_definition` for both `function f {}` and `f() {}` forms.
- Doc comment extraction — captures `///`, `/** */`, `# `, `// ` preceding definitions.
  Included in JSON output and embedding text for better semantic search.
- `doc` field in `QueryResult` (skipped when absent via `skip_serializing_if`).

### Changed
- Candle 0.8 compatibility — `mean_pool` and `normalize` use `Tensor::full` instead of
  `expand`/`repeat` to work around candle 0.8 non-materialized view bug.
- Index serialization uses bincode (`.sift/index.bin`) instead of JSON.

## [0.1.1] — 2026-05-20

### Fixed
- Repository URLs corrected to `github.com/rsarv3006/sift`.

## [0.1.0] — 2026-05-20

### Added
- Initial release.
- Structural codebase index via tree-sitter (Rust, Python, JavaScript, TypeScript, TSX,
  Go, C, C++, Java, Ruby, Zig).
- Cross-file call graph and import resolution.
- CLI commands: `sift index`, `sift query`, `sift skill`.
- Semantic embedding layer with candle (local) and OpenAI-compatible API fallback.
- All query types: `define`, `calls`, `callees`, `implements`, `file`, `files`,
  `symbols matching`, `semantic`, `imports`, `importers`.
- Agentic benchmark harness (25 structural + 20 semantic tasks).
- CI via GitHub Actions.
- `.claude/skills/sift.mdc` for LLM auto-discovery.
