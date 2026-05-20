# Changelog

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
