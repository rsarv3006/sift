use anyhow::Result;
use clap::Parser;
use rayon::prelude::*;
use sift::embed::{AutoEmbedder, EmbedConfig, Embedder};
use sift::index::CodeIndex;
use sift::parser::{parse_file, LanguageId};
use sift::query::QueryEngine;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Parser)]
#[command(name = "sift", version, about = "Structural codebase index for LLM tooling")]
enum Cli {
    /// Build a structural index of a codebase
    Index {
        /// Path to the codebase
        path: String,
        /// Output path for the index (default: .sift/index.bin)
        #[arg(short, long)]
        output: Option<String>,
        /// Enable semantic embeddings (uses SIFT_EMBED_* env vars)
        #[arg(long)]
        embed: bool,
    },
    /// Query the structural index
    Query {
        /// Query string: define <name>, calls <name>, callees <name>,
        /// symbols matching <pattern>, file <path>, files, semantic <query>
        query: String,
        /// Path to the index (default: .sift/index.bin)
        #[arg(short, long)]
        index: Option<String>,
        /// Enable semantic search (uses SIFT_EMBED_* env vars)
        #[arg(long)]
        embed: bool,
    },
    /// Output the LLM tool definition for this tool
    Skill,
}

fn main() -> Result<()> {
    match Cli::parse() {
        Cli::Index { path, output, embed } => cmd_index(&path, output.as_deref(), embed),
        Cli::Query { query, index, embed } => cmd_query(&query, index.as_deref(), embed),
        Cli::Skill => cmd_skill(),
    }
}

fn cmd_index(path: &str, output: Option<&str>, embed: bool) -> Result<()> {
    let start = Instant::now();

    let root = Path::new(path);
    if !root.exists() {
        anyhow::bail!("path does not exist: {}", path);
    }
    let root = root.canonicalize()?;
    let out_path = resolve_output_path(&root, output);

    let embedder: Option<AutoEmbedder> = if embed {
        let config = EmbedConfig::load();
        let has_local = cfg!(feature = "candle");
        let has_api = config.api_key.is_some()
            || std::env::var("OPENAI_API_KEY").is_ok()
            || config.api_url.is_some();
        if !has_local && !has_api {
            eprintln!("warn: --embed specified but no embedding backend configured");
            eprintln!("  Set SIFT_EMBED_API_KEY for OpenAI/API embedding, or");
            eprintln!("  set SIFT_EMBED_API_URL for a local API (e.g. Ollama), or");
            eprintln!("  build with `--features candle` for local embeddings");
        }
        match AutoEmbedder::new(&config) {
            Ok(e) => Some(e),
            Err(e) => {
                eprintln!("warn: embedding disabled: {}", e);
                None
            }
        }
    } else {
        None
    };

    println!("Indexing {}...", root.display());

    let walk = ignore::WalkBuilder::new(&root)
        .standard_filters(true)
        .build();

    let mut files: Vec<PathBuf> = Vec::new();
    for entry in walk {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("  warn: skipping entry: {:#}", e);
                continue;
            }
        };
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            let path = entry.path();
            if path.components().any(|c| c.as_os_str() == "target") {
                continue;
            }
            if LanguageId::from_path(path).is_some() {
                files.push(path.to_path_buf());
            }
        }
    }

    if files.is_empty() {
        anyhow::bail!("no supported source files found in {}", root.display());
    }
    println!("Found {} parseable files", files.len());

    let results: Vec<_> = files
        .par_iter()
        .filter_map(|path| match parse_file(path) {
            Ok(parsed) => Some(parsed),
            Err(e) => {
                eprintln!("  warn: {}: {:#}", path.display(), e);
                None
            }
        })
        .collect();

    if results.is_empty() {
        anyhow::bail!("no files could be parsed in {}", root.display());
    }
    println!("Parsed {} files in {:?}", results.len(), start.elapsed());

    let index = CodeIndex::build(results, &root, embedder.as_ref().map(|e| e as &dyn Embedder));
    index.save(&out_path)?;

    let embedded = index.symbols.iter().filter(|s| s.embedding.is_some()).count();
    println!(
        "Index saved to {} ({} symbols, {} calls, {} imports, {} embedded) in {:?}",
        out_path.display(),
        index.symbols.len(),
        index.calls.len(),
        index.imports.len(),
        embedded,
        start.elapsed(),
    );

    Ok(())
}

fn cmd_query(query_str: &str, index: Option<&str>, embed: bool) -> Result<()> {
    let query_str = query_str.trim();
    if query_str.is_empty() {
        anyhow::bail!("query string is empty — try: define <name>, calls <name>, file <path>, etc.");
    }

    let index_path = if let Some(p) = index {
        PathBuf::from(p)
    } else {
        PathBuf::from(".sift/index.bin")
    };

    if !index_path.exists() {
        anyhow::bail!(
            "index not found at {} — run `sift index <path>` first",
            index_path.display()
        );
    }
    let index = CodeIndex::load(&index_path)?;
    let engine = if embed {
        let config = EmbedConfig::load();
        match AutoEmbedder::new(&config) {
            Ok(e) => QueryEngine::with_embedder(&index, Box::new(e)),
            Err(e) => {
                eprintln!("warn: semantic search disabled: {}", e);
                QueryEngine::new(&index)
            }
        }
    } else {
        QueryEngine::new(&index)
    };
    let results = engine.execute(query_str);

    if results.is_empty() {
        println!("No results");
        return Ok(());
    }

    let json = serde_json::to_string_pretty(&results)?;
    println!("{json}");

    Ok(())
}

fn cmd_skill() -> Result<()> {
    let skill = r#"# sift: Codebase Structural Index

sift builds a structural index of your codebase (symbols, call graphs, imports)
using tree-sitter. It supports Rust, Python, JavaScript, TypeScript, TSX, Go,
C, C++, Java, Ruby, Zig, and Bash — with no API keys or network required.

## When to use sift

Use sift instead of reading files directly when you need to:
- Find where a symbol is defined
- Trace callers/callees of a function
- Find all implementations of an interface/trait
- Discover code relationships across files
- Search for symbols by name pattern
- Find relevant code by describing what it does (semantic search, requires --embed)

sift returns *minimal structured data* — just enough to understand relationships,
not full source code. If you need the actual implementation, read the file directly.

## Available commands

### `sift query "define <name>"`
Find all definitions whose name matches. Returns symbol kind, file, and line range.

### `sift query "calls <name>"`
Find all callers of functions/methods named <name>. Returns file and line per call site.

### `sift query "callees <name>"`
Find all functions called by definitions named <name>.

### `sift query "implements <name>"`
Find all implementations of traits/interfaces named <name>.

### `sift query "file <path>"`
List all symbols defined in a given file.

### `sift query "symbols matching <pattern>"`
Case-insensitive substring search across all symbol names.

### `sift query "semantic <description>"`
Semantic search using embeddings (requires --embed on both index and query).
Embeds the description and returns top-10 symbols ranked by cosine similarity.
Each result includes a `score` field (0.0-1.0). Example:
  sift index --embed .
  sift query --embed "semantic calculate monthly revenue"

Config via env vars or config file (later wins):
  - `~/.config/sift/config.toml`
  - `.sift/config.toml` (project-level)
  - `SIFT_EMBED_*` env vars

  Example `.sift/config.toml`:
    [embed]
    backend = "api"
    api_url = "http://localhost:11434/v1/embeddings"
    api_model = "nomic-embed-text"

  Env var reference:
    SIFT_EMBED_API_KEY       # API key (optional with local backends like Ollama)
    SIFT_EMBED_API_URL       # default: https://api.openai.com/v1/embeddings
    SIFT_EMBED_API_MODEL     # default: text-embedding-3-small
    SIFT_EMBED_BACKEND       # "auto" (default), "api", or "local"
    SIFT_EMBED_MODEL_PATH    # path to local model files (candle feature)

  If no backend is available, sift prints a warning explaining how to configure one.

### `sift query "files"`
List all indexed files (relative paths).

## JSON output format

```json
[
  {"type": "definition", "name": "...", "kind": "function", "file": "src/foo.rs", "line": 10, "end_line": 42, "doc": "/// Doc comment text"},
  {"type": "call", "caller": "foo", "callee": "bar", "file": "src/foo.rs", "line": 15},
  {"type": "semantic", "name": "...", "kind": "function", "file": "src/bar.rs", "line": 5, "end_line": 20, "score": 0.92, "doc": "/// Doc comment text"}
]
```
"#;
    println!("{skill}");
    Ok(())
}

fn resolve_output_path(root: &Path, output: Option<&str>) -> PathBuf {
    if let Some(p) = output {
        PathBuf::from(p)
    } else {
        let dir = root.join(".sift");
        let _ = std::fs::create_dir_all(&dir);
        dir.join("index.bin")
    }
}
