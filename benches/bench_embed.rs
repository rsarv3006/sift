//! Semantic embedding benchmark.
//!
//! For each fixture directory:
//!   1. Index with `--embed` (requires embed backend — see SIFT_EMBED_* env vars)
//!   2. Run `sift query --embed "..."` tasks from `tasks_embed.json`
//!   3. Measure output bytes vs naive cost (entire codebase)
//!   4. Verify expected symbols appear in top-K results

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

#[derive(serde::Deserialize)]
struct TaskFile {
    #[allow(dead_code)]
    description: String,
    tasks: Vec<Task>,
}

#[derive(serde::Deserialize)]
struct Task {
    id: String,
    description: Option<String>,
    query: String,
    expected: Option<Expected>,
    expected_any: Option<Vec<Expected>>,
    expected_names: Option<Vec<String>>,
    expected_any_names: Option<Vec<String>>,
    expected_min: Option<usize>,
}

#[derive(Debug, serde::Deserialize)]
struct Expected {
    #[serde(rename = "type")]
    typ: Option<String>,
    name: Option<String>,
    kind: Option<String>,
    file: Option<String>,
}

fn main() {
    let root = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string()),
    );
    let fixtures_dir = root.join("bench-fixtures");
    let sift_bin = find_sift_bin(&root);

    // Check embedder is configured before running
    println!("# sift Embedding Benchmark\n");
    println!("Requires embed backend: set SIFT_EMBED_BACKEND=api and SIFT_EMBED_API_URL\n");

    match test_embedder(&sift_bin) {
        Ok(msg) => println!("Embedder check: {}\n", msg),
        Err(e) => {
            eprintln!("error: {}.\nSet SIFT_EMBED_BACKEND=api and SIFT_EMBED_API_URL (e.g. http://localhost:11434/v1/embeddings for Ollama), optionally SIFT_EMBED_API_KEY.", e);
            std::process::exit(1);
        }
    }

    let mut all_tasks = 0u64;
    let mut passed = 0u64;
    let mut total_sift_bytes = 0u64;
    let mut total_naive_bytes = 0u64;

    let mut entries: Vec<_> = fs::read_dir(&fixtures_dir)
        .expect("bench-fixtures/ not found")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    entries.sort_by_key(|e| e.path());

    for entry in &entries {
        let fixture = entry.path();
        let name = fixture.file_name().unwrap().to_string_lossy();
        let tasks_path = fixture.join("tasks_embed.json");

        if !tasks_path.exists() {
            eprintln!("  skipping {}: no tasks_embed.json", name);
            continue;
        }

        let src_dir = fixture.join("src");
        let source_bytes = total_source_bytes(&src_dir);

        // Build sift index WITH embeddings
        let index_dir = fixture.join(".sift");
        let index_path = index_dir.join("index.bin");
        if index_path.exists() {
            let _ = fs::remove_dir_all(&index_dir);
        }

        let start = Instant::now();
        let index_out = Command::new(&sift_bin)
            .arg("index")
            .arg("--embed")
            .arg(&fixture)
            .output()
            .expect("sift index --embed failed");
        let index_time = start.elapsed();

        if !index_out.status.success() {
            eprintln!(
                "  FAIL index --embed {}: {}",
                name,
                String::from_utf8_lossy(&index_out.stderr)
            );
            continue;
        }

        let tasks: TaskFile = serde_json::from_str(&fs::read_to_string(&tasks_path).unwrap())
            .expect("invalid tasks_embed.json");

        let embedded_count = count_embedded(&index_path);

        println!("## {}\n", name);
        println!(
            "  - Source files: {}  |  Total bytes: {}  |  Symbols embedded: {}",
            count_files(&src_dir),
            source_bytes,
            embedded_count
        );
        println!("  - Tasks: {}  |  Index time: {:?}\n", tasks.tasks.len(), index_time);
        println!(
            "| Task | Result | Sift out (bytes) | Naive cost (bytes) | Savings |
|------|--------|-------------------|--------------------|---------|"
        );

        for task in &tasks.tasks {
            let task_start = Instant::now();
            let query_out = Command::new(&sift_bin)
                .arg("query")
                .arg("--embed")
                .arg(&task.query)
                .current_dir(&fixture)
                .output()
                .expect("sift query --embed failed");
            let _query_time = task_start.elapsed();

            let sift_output = String::from_utf8_lossy(&query_out.stdout);
            let sift_bytes = sift_output.len() as u64;
            let ok = query_out.status.success();

            let correct = if !ok {
                false
            } else {
                verify_task(task, &sift_output)
            };

            if correct {
                passed += 1;
            }
            all_tasks += 1;

            // Naive cost: agent must read entire codebase to understand semantics
            let naive_cost = source_bytes;
            let savings = if naive_cost > 0 {
                naive_cost as f64 / sift_bytes.max(1) as f64
            } else {
                0.0
            };

            total_sift_bytes += sift_bytes;
            total_naive_bytes += naive_cost;

            let status = if correct { "✅" } else { "❌" };
            let desc = task.description.as_deref().unwrap_or(&task.id);
            println!(
                "| {} | {} | {} | {} | {:.0}x |",
                desc, status, sift_bytes, naive_cost, savings
            );
        }
        println!();
    }

    // Summary
    println!("## Summary\n");
    let total_savings = if total_sift_bytes > 0 {
        total_naive_bytes as f64 / total_sift_bytes as f64
    } else {
        0.0
    };
    println!(
        "| Metric | Value |
|--------|-------|
| Tasks | {}/{} passed |
| Total sift output | {} bytes |
| Total naive cost | {} bytes |
| Avg savings factor | {:.0}x |
| Est. tokens saved | {} |",
        passed,
        all_tasks,
        total_sift_bytes,
        total_naive_bytes,
        total_savings,
        (total_naive_bytes as f64 / 4.0) as u64 - (total_sift_bytes as f64 / 4.0) as u64
    );

    if passed < all_tasks {
        println!("\n⚠️  Some tasks failed — see ❌ rows above.");
        std::process::exit(1);
    }
}

fn test_embedder(sift_bin: &Path) -> Result<String, String> {
    // Create a tiny temp dir with a single Rust file
    let tmp = std::env::temp_dir().join("sift_embed_test");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).map_err(|e| format!("mkdir: {}", e))?;
    fs::write(
        tmp.join("test.rs"),
        "pub fn foo() -> i32 { 42 }"
    ).map_err(|e| format!("write: {}", e))?;

    let out = Command::new(sift_bin)
        .arg("index")
        .arg("--embed")
        .arg(&tmp)
        .output()
        .map_err(|e| format!("exec: {}", e))?;

    let _ = fs::remove_dir_all(&tmp);

    if out.status.success() {
        // Extract embedded count from status line
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let all = format!("{}{}", stdout, stderr);
        if let Some(line) = all.lines().find(|l| l.contains("embedded")) {
            Ok(line.trim().to_string())
        } else {
            Ok("embedder works".to_string())
        }
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(format!("embedder failed: {}", stderr.trim()))
    }
}

fn count_embedded(index_path: &Path) -> usize {
    if !index_path.exists() {
        return 0;
    }
    let bytes = fs::read(index_path).unwrap_or_default();
    match bincode::deserialize::<sift::index::CodeIndex>(&bytes) {
        Ok(idx) => idx.symbols.iter().filter(|s| s.embedding.is_some()).count(),
        Err(_) => 0,
    }
}

fn find_sift_bin(root: &Path) -> PathBuf {
    let status = Command::new("cargo")
        .arg("build")
        .arg("--bin")
        .arg("sift")
        .arg("--features")
        .arg("candle")
        .current_dir(root)
        .status()
        .expect("cargo build failed");
    assert!(status.success(), "cargo build failed");
    root.join("target").join("debug").join("sift")
}

fn total_source_bytes(dir: &Path) -> u64 {
    let mut total = 0u64;
    if dir.exists() {
        walk_files(dir, &mut |path| {
            if let Ok(meta) = path.metadata() {
                total += meta.len();
            }
        });
    }
    total
}

fn count_files(dir: &Path) -> usize {
    let mut count = 0;
    if dir.exists() {
        walk_files(dir, &mut |_| count += 1);
    }
    count
}

fn walk_files(dir: &Path, f: &mut dyn FnMut(&Path)) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk_files(&path, f);
            } else if path.is_file() {
                f(&path);
            }
        }
    }
}

fn verify_task(task: &Task, sift_output: &str) -> bool {
    let trimmed = sift_output.trim();
    if trimmed.is_empty() || trimmed == "[]" {
        eprintln!("    [{}] empty result", task.id);
        return false;
    }

    let val: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("    [{}] parse error: {}", task.id, e);
            return false;
        }
    };
    let arr = match &val {
        serde_json::Value::Array(a) => a,
        _ => return false,
    };

    if let Some(min) = task.expected_min {
        if arr.len() < min {
            eprintln!(
                "    [{}] expected >= {} results, got {}",
                task.id,
                min,
                arr.len()
            );
            return false;
        }
    }

    if let Some(expected) = &task.expected {
        return matches_entry(arr, expected);
    }

    if let Some(expected_list) = &task.expected_any {
        for exp in expected_list {
            if arr.iter().any(|entry| entry_matches(entry, exp)) {
                return true;
            }
        }
        eprintln!("    [{}] no expected_any entry matched: {:?}", task.id, expected_list);
        return false;
    }

    if let Some(expected_names) = &task.expected_names {
        let names: Vec<&str> = arr
            .iter()
            .filter_map(|v| v.get("name"))
            .filter_map(|v| v.as_str())
            .collect();
        for n in expected_names {
            if !names.contains(&n.as_str()) {
                eprintln!("    [{}] expected name '{}' not found in {:?}", task.id, n, names);
                return false;
            }
        }
        return true;
    }

    if let Some(expected_any_names) = &task.expected_any_names {
        let names: Vec<&str> = arr
            .iter()
            .filter_map(|v| v.get("name"))
            .filter_map(|v| v.as_str())
            .collect();
        for n in expected_any_names {
            if names.contains(&n.as_str()) {
                return true;
            }
        }
        eprintln!("    [{}] none of expected_any_names found in {:?}", task.id, names);
        return false;
    }

    true
}

fn matches_entry(arr: &[serde_json::Value], expected: &Expected) -> bool {
    if arr.is_empty() {
        return false;
    }
    arr.iter().any(|entry| entry_matches(entry, expected))
}

fn entry_matches(entry: &serde_json::Value, expected: &Expected) -> bool {
    if let Some(typ) = &expected.typ {
        if entry.get("type").and_then(|v| v.as_str()) != Some(typ.as_str()) {
            return false;
        }
    }
    if let Some(name) = &expected.name {
        if entry.get("name").and_then(|v| v.as_str()) != Some(name.as_str()) {
            return false;
        }
    }
    if let Some(kind) = &expected.kind {
        if entry.get("kind").and_then(|v| v.as_str()) != Some(kind.as_str()) {
            return false;
        }
    }
    if let Some(file) = &expected.file {
        if entry.get("file").and_then(|v| v.as_str()) != Some(file.as_str()) {
            return false;
        }
    }
    true
}
