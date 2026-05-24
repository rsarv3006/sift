use anyhow::Result;
use clap::Parser;
use rayon::prelude::*;
use sift::embed::{AutoEmbedder, EmbedConfig, Embedder};
use sift::index::{is_relevant_source_event, CodeIndex};
use sift::parser::{parse_file, LanguageId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};
use std::time::{Duration, Instant, UNIX_EPOCH};

use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};

fn report_parse_speed(
    parsed: &[sift::parser::ParsedFile],
    wall_clock: Duration,
) {
    let mut per_lang: HashMap<LanguageId, (usize, Duration)> = HashMap::new();
    for pf in parsed {
        let dur = pf.parse_duration.unwrap_or(Duration::ZERO);
        let entry = per_lang.entry(pf.language).or_default();
        entry.0 += 1;
        entry.1 += dur;
    }

    let mut sorted: Vec<_> = per_lang.into_iter().collect();
    sorted.sort_by_key(|a| std::cmp::Reverse(a.1 .1));

    for (lang, (count, total)) in &sorted {
        let avg_ms = total.as_secs_f64() * 1000.0 / (*count).max(1) as f64;
        eprintln!(
            "  {:>8}: {:>4} files, {:>7.1} ms avg, {:>7.2}s cpu",
            format!("{:?}", lang).to_lowercase(),
            count,
            avg_ms,
            total.as_secs_f64(),
        );
    }

    let total = parsed.len();
    let rate = if wall_clock.as_secs_f64() > 0.0 {
        total as f64 / wall_clock.as_secs_f64()
    } else {
        0.0
    };
    eprintln!(
        "  Total: {} files in {:.3}s wall ({:.1} files/s)",
        total,
        wall_clock.as_secs_f64(),
        rate,
    );
}

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
        /// Maximum number of results to return
        #[arg(long, default_value_t = 100)]
        limit: usize,
        /// Number of results to skip before returning
        #[arg(long, default_value_t = 0)]
        offset: usize,
        /// Enable semantic search (uses SIFT_EMBED_* env vars)
        #[arg(long)]
        embed: bool,
    },
    /// Watch a codebase and automatically re-index on changes
    Watch {
        /// Path to the codebase
        path: Option<String>,
        /// Enable semantic embeddings on re-index (slower)
        #[arg(long)]
        embed: bool,
        /// Run as a daemon (fork to background)
        #[arg(short = 'd', long)]
        daemonize: bool,
    },
    /// Output the LLM tool definition for this tool
    Skill,
}

fn main() -> Result<()> {
    match Cli::parse() {
        Cli::Index { path, output, embed } => cmd_index(&path, output.as_deref(), embed),
        Cli::Query { query, index, limit, offset, embed } => cmd_query(&query, index.as_deref(), limit, offset, embed),
        Cli::Watch { path, embed, daemonize } => cmd_watch(path.as_deref(), embed, daemonize),
        Cli::Skill => cmd_skill(),
    }
}

fn collect_source_files(root: &Path) -> (Vec<PathBuf>, HashMap<PathBuf, u64>) {
    let mut files = Vec::new();
    let mut mtimes = HashMap::new();
    let walk = ignore::WalkBuilder::new(root)
        .standard_filters(true)
        .build();
    for entry in walk {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        if path.components().any(|c| c.as_os_str() == "target") {
            continue;
        }
        if LanguageId::from_path(path).is_none() {
            continue;
        }
        files.push(path.to_path_buf());
        if let Ok(meta) = path.metadata() {
            if let Ok(mtime) = meta.modified() {
                if let Ok(dur) = mtime.duration_since(UNIX_EPOCH) {
                    mtimes.insert(path.to_path_buf(), dur.as_millis() as u64);
                }
            }
        }
    }
    (files, mtimes)
}

/// Incrementally re-index: parse changed files and update the index in-place.
/// `changed` is the list of known-changed file paths (from events or mtime comparison).
/// `current_mtimes` is the full current mtimes map (used to update `index.file_mtimes`).
fn reindex_incremental(
    index: &mut CodeIndex,
    changed: &[PathBuf],
    current_mtimes: &HashMap<PathBuf, u64>,
    embedder: Option<&dyn Embedder>,
) -> (bool, Duration) {
    let start = Instant::now();

    if changed.is_empty() {
        return (false, start.elapsed());
    }

    // Deduplicate changed paths
    let mut seen = std::collections::HashSet::new();
    let changed: Vec<PathBuf> = changed
        .iter()
        .filter(|p| seen.insert(p.as_path()))
        .cloned()
        .collect();

    // Capture embeddings for changed files before removal
    let old_embeddings: HashMap<(String, PathBuf, usize), Vec<f32>> = index
        .symbols
        .iter()
        .filter(|s| changed.iter().any(|p| p == &s.file))
        .filter_map(|s| {
            s.embedding
                .clone()
                .map(|emb| ((s.name.clone(), s.file.clone(), s.line), emb))
        })
        .collect();

    let new_parsed: Vec<_> = changed
        .par_iter()
        .filter_map(|path| match parse_file(path) {
            Ok(p) => Some(p),
            Err(e) => {
                eprintln!("  warn: {}: {:#}", path.display(), e);
                None
            }
        })
        .collect();

    let changed_parsed_count = new_parsed.len();

    index.apply_changes(&changed, &new_parsed);
    index.file_mtimes = current_mtimes.clone();

    if let Some(embedder) = embedder {
        index.compute_missing_embeddings(embedder);
    } else if !old_embeddings.is_empty() {
        let changed_set: std::collections::HashSet<&Path> =
            changed.iter().map(|p| p.as_path()).collect();
        for sym in &mut index.symbols {
            if sym.embedding.is_none() && changed_set.contains(sym.file.as_path()) {
                if let Some(emb) =
                    old_embeddings.get(&(sym.name.clone(), sym.file.clone(), sym.line))
                {
                    sym.embedding = Some(emb.clone());
                }
            }
        }
    }

    eprintln!(
        "[sift] auto-re-indexed: {} changed ({} new parsed) in {:?}",
        changed.len(),
        changed_parsed_count,
        start.elapsed(),
    );

    (true, start.elapsed())
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

    let (files, current_mtimes) = collect_source_files(&root);
    if files.is_empty() {
        anyhow::bail!("no supported source files found in {}", root.display());
    }

    // Try incremental re-index if an existing index is present
    if let Ok(mut old) = CodeIndex::load(&out_path) {
        let (_unchanged, changed) = old.classify_files(&current_mtimes);
        if changed.is_empty() {
            println!(
                "Index is up to date ({} files, {} symbols, {} calls, {} imports)",
                old.files.len(),
                old.symbols.len(),
                old.calls.len(),
                old.imports.len(),
            );
            return Ok(());
        }

        reindex_incremental(
            &mut old,
            &changed,
            &current_mtimes,
            embedder.as_ref().map(|e| e as &dyn Embedder),
        );

        old.save(&out_path)?;

        let embedded = old.symbols.iter().filter(|s| s.embedding.is_some()).count();
        println!(
            "Index saved to {} ({} symbols, {} calls, {} imports, {} embedded) in {:?}",
            out_path.display(),
            old.symbols.len(),
            old.calls.len(),
            old.imports.len(),
            embedded,
            start.elapsed(),
        );
        return Ok(());
    }

    // Fresh index
    println!("Indexing {}...", root.display());
    println!("Found {} parseable files", files.len());

    let parse_start = Instant::now();
    let parsing_done = Arc::new(AtomicBool::new(false));
    let done = parsing_done.clone();
    let parsed_count = Arc::new(AtomicUsize::new(0));
    let count = parsed_count.clone();
    let total = files.len();
    let progress = std::thread::spawn(move || {
        while !done.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_secs(10));
            if !done.load(Ordering::Relaxed) {
                let n = count.load(Ordering::Relaxed);
                eprint!("\r  parsing... ({}/{}, {}s elapsed)", n, total, parse_start.elapsed().as_secs());
            }
        }
    });

    let parsed: Vec<_> = files
        .par_iter()
        .filter_map(|path| {
            let result = parse_file(path);
            parsed_count.fetch_add(1, Ordering::Relaxed);
            match result {
                Ok(p) => Some(p),
                Err(e) => {
                    eprintln!("\n  warn: {}: {:#}", path.display(), e);
                    None
                }
            }
        })
        .collect();

    parsing_done.store(true, Ordering::Relaxed);
    let _ = progress.join();
    eprint!("\r  parsing... ({}/{}, {}s elapsed)\n", parsed.len(), total, parse_start.elapsed().as_secs());

    if parsed.is_empty() {
        anyhow::bail!("no files could be parsed in {}", root.display());
    }
    eprintln!("Parsing speed:");
    report_parse_speed(&parsed, parse_start.elapsed());
    println!("Parsed {} files in {:?}", parsed.len(), start.elapsed());

    let mut index = CodeIndex::build(
        parsed,
        &root,
        embedder.as_ref().map(|e| e as &dyn Embedder),
    );
    index.file_mtimes = current_mtimes;
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

fn cmd_query(query_str: &str, index: Option<&str>, limit: usize, offset: usize, embed: bool) -> Result<()> {
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

    let mut index = CodeIndex::load(&index_path)?;

    // Check staleness and auto-re-index if needed
    let root = index.root.clone();
    let (_files, current_mtimes) = collect_source_files(&root);
    let (_unchanged, changed) = index.classify_files(&current_mtimes);

    if !changed.is_empty() {
        let reindex_start = Instant::now();
        reindex_incremental(
            &mut index,
            &changed,
            &current_mtimes,
            None, // structural re-index only on query
        );
        index.save(&index_path)?;
        eprintln!(
            "[sift] auto-re-index completed in {:?}",
            reindex_start.elapsed(),
        );
    }

    let engine = if embed {
        let config = EmbedConfig::load();
        match AutoEmbedder::new(&config) {
            Ok(e) => sift::query::QueryEngine::with_embedder(&index, Box::new(e)),
            Err(e) => {
                eprintln!("warn: semantic search disabled: {}", e);
                sift::query::QueryEngine::new(&index)
            }
        }
    } else {
        sift::query::QueryEngine::new(&index)
    };
    let results = engine.execute(query_str);
    let total = results.len();

    let slice: Vec<_> = results
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect();

    if slice.is_empty() {
        println!("No results");
        return Ok(());
    }

    if total > slice.len() {
        let end = offset + slice.len();
        eprintln!(
            "[sift] showing results {}-{} of {}",
            offset + 1,
            end,
            total
        );
    }

    let json = serde_json::to_string_pretty(&slice)?;
    println!("{json}");

    Ok(())
}

fn cmd_watch(path: Option<&str>, embed: bool, daemonize: bool) -> Result<()> {
    let root = Path::new(path.unwrap_or("."));
    if !root.exists() {
        anyhow::bail!("path does not exist: {}", root.display());
    }
    let root = root.canonicalize()?;
    let out_path = resolve_output_path(&root, None);

    let mut embedder: Option<AutoEmbedder> = if embed {
        let config = EmbedConfig::load();
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

    let (files, current_mtimes) = collect_source_files(&root);
    if files.is_empty() {
        anyhow::bail!("no supported source files found in {}", root.display());
    }

    // Initial index
    let index: CodeIndex = if let Ok(mut old) = CodeIndex::load(&out_path) {
        let (_unchanged, changed) = old.classify_files(&current_mtimes);
        if changed.is_empty() {
            println!(
                "Loaded existing index ({} files, {} symbols)",
                old.files.len(),
                old.symbols.len()
            );
            old
        } else {
            let rebuild_start = Instant::now();
            reindex_incremental(
                &mut old,
                &changed,
                &current_mtimes,
                embedder.as_ref().map(|e| e as &dyn Embedder),
            );
            old.save(&out_path)?;
            println!(
                "Re-indexed ({} symbols) in {:.3}s",
                old.symbols.len(),
                rebuild_start.elapsed().as_secs_f64(),
            );
            old
        }
    } else {
        let index_start = Instant::now();
        println!("Indexing {}...", root.display());
        let parsed: Vec<_> = files
            .par_iter()
            .filter_map(|path| parse_file(path).ok())
            .collect();
        let mut idx = CodeIndex::build(
            parsed,
            &root,
            embedder.as_ref().map(|e| e as &dyn Embedder),
        );
        idx.file_mtimes = current_mtimes.clone();
        idx.save(&out_path)?;
        println!(
            "Initial index complete ({} symbols) in {:.3}s",
            idx.symbols.len(),
            index_start.elapsed().as_secs_f64(),
        );
        idx
    };

    // Set up file watcher
    let (tx, rx) = mpsc::channel();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        Config::default(),
    )
    .map_err(|e| anyhow::anyhow!("failed to create file watcher: {}", e))?;

    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|e| anyhow::anyhow!("failed to watch {}: {}", root.display(), e))?;

    if daemonize {
        let pid_file = out_path.with_file_name("watcher.pid");
        if let Err(e) = std::fs::write(&pid_file, format!("{}\n", std::process::id())) {
            eprintln!("[sift] warning: failed to write PID file: {}", e);
        }
        eprintln!(
            "[sift] daemonizing (PID {})...",
            std::process::id(),
        );
        // Fork to background: `daemon(0, 0)` changes to /, closes stdio
        let ret = unsafe { libc::daemon(0, 0) };
        if ret != 0 {
            anyhow::bail!("daemonization failed: {}", std::io::Error::last_os_error());
        }
        // After fork: drop the embedder (reqwest client threads don't survive fork).
        // Embeddings are preserved from old index via preserve_embeddings in the event loop.
        drop(embedder);
        embedder = None;
    } else {
        eprintln!(
            "[sift] watching {} for changes... (Ctrl+C to stop)",
            root.display(),
        );
    }

    // Event loop with debounce: wait for a quiet 500ms period before re-indexing.
    // Re-index runs in a background thread so file events are still collected.
    // Uses file events to track changed paths instead of walking the filesystem.
    let debounce = Duration::from_millis(500);
    let mut pending_since: Option<Instant> = None;
    let mut pending_paths: Vec<PathBuf> = Vec::new();
    let reindexing = Arc::new(AtomicBool::new(false));
    let (tx_done, rx_done) = mpsc::channel::<(CodeIndex, HashMap<PathBuf, u64>)>();
    let embedder_arc = embedder.map(Arc::new);
    let mut index = Some(index);

    loop {
        // Drain completed re-index results (non-blocking)
        while let Ok((new_index, _new_mtimes)) = rx_done.try_recv() {
            index = Some(new_index);
        }

        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Ok(event)) => {
                if is_relevant_source_event(&event) {
                    let was_pending = pending_since.is_some();
                    if !was_pending {
                        let paths: Vec<_> = event
                            .paths
                            .iter()
                            .filter_map(|p| p.file_name())
                            .map(|n| n.to_string_lossy().to_string())
                            .collect();
                        eprintln!(
                            "[sift] change detected: {} ({:?})",
                            paths.join(", "),
                            event.kind,
                        );
                        pending_since = Some(Instant::now());
                    }
                    // Accumulate paths regardless — don't re-walk the tree
                    for path in &event.paths {
                        if !pending_paths.contains(path) {
                            pending_paths.push(path.clone());
                        }
                    }
                }
            }
            Ok(Err(e)) => eprintln!("[sift] watch error: {}", e),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Some(since) = pending_since {
                    if since.elapsed() >= debounce
                        && !reindexing.load(Ordering::Relaxed)
                    {
                        // Take ownership of the index — move it into the worker thread
                        // and get it back via the channel. This avoids cloning 3.2M symbols.
                        if let Some(mut idx) = index.take() {
                            eprintln!("[sift] re-indexing...");
                            reindexing.store(true, Ordering::Relaxed);
                            let changed_paths = std::mem::take(&mut pending_paths);
                            let out_path_clone = out_path.clone();
                            let tx_done = tx_done.clone();
                            let reindexing_flag = reindexing.clone();
                            let embedder_thread = embedder_arc.clone();

                            std::thread::spawn(move || {
                                // Build current_mtimes from stored data + stat of changed files.
                                // Avoids walking the entire filesystem tree (saves ~237ms on 65K files).
                                let mut current_mtimes = idx.file_mtimes.clone();
                                for path in &changed_paths {
                                    match std::fs::metadata(path)
                                        .and_then(|m| m.modified())
                                    {
                                        Ok(mtime) => {
                                            let ms = mtime
                                                .duration_since(UNIX_EPOCH)
                                                .unwrap_or_default()
                                                .as_millis()
                                                as u64;
                                            current_mtimes.insert(path.clone(), ms);
                                        }
                                        Err(_) => {
                                            current_mtimes.remove(path);
                                        }
                                    }
                                }
                                let (rebuilt, _dur) = reindex_incremental(
                                    &mut idx,
                                    &changed_paths,
                                    &current_mtimes,
                                    embedder_thread
                                        .as_deref()
                                        .map(|e| e as &dyn Embedder),
                                );
                                if rebuilt {
                                    if let Err(e) = idx.save(&out_path_clone) {
                                        eprintln!(
                                            "[sift] failed to save index: {}",
                                            e
                                        );
                                    }
                                }
                                let _ = tx_done.send((idx, current_mtimes));
                                reindexing_flag.store(false, Ordering::Relaxed);
                            });
                            pending_since = None;
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                eprintln!("[sift] watcher disconnected");
                break Ok(());
            }
        }
    }
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
    SIFT_EMBED_API_URL       # default: https://api.openapi.com/v1/embeddings
    SIFT_EMBED_API_MODEL     # default: text-embedding-3-small
    SIFT_EMBED_BACKEND       # "auto" (default), "api", or "local"
    SIFT_EMBED_MODEL_PATH    # path to local model files (candle feature)

  If no backend is available, sift prints a warning explaining how to configure one.

### `sift query "files"`
List all indexed files (relative paths).

### Pagination

All query commands support `--limit` and `--offset` for paginating
large result sets:
  sift query --limit 5 --offset 10 "define Parser"

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
