//! Agentic benchmark: sift vs. naive source-code search.
//!
//! For each fixture directory:
//!   1. Compute total source bytes (what naive grep must read)
//!   2. Index the directory with sift
//!   3. Run each task query, measure output bytes, verify correctness
//!   4. Print a comparison table

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

#[allow(dead_code)]
#[derive(serde::Deserialize)]
struct TaskFile {
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
    expected_symbols: Option<Vec<String>>,
    expected_imports: Option<Vec<ExpectedImport>>,
    expected_min: Option<usize>,
    /// Pagination limit for this task query.
    limit: Option<usize>,
    /// Pagination offset for this task query.
    offset: Option<usize>,
}

#[derive(serde::Deserialize)]
struct Expected {
    #[serde(rename = "type")]
    typ: Option<String>,
    name: Option<String>,
    kind: Option<String>,
    file: Option<String>,
    line: Option<usize>,
    caller: Option<String>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct ExpectedImport {
    symbol: Option<String>,
    resolved: Option<bool>,
}

fn main() {
    let root = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string()),
    );
    let fixtures_dir = root.join("bench-fixtures");
    let sift_bin = find_sift_bin(&root);

    println!("# sift Agentic Benchmark\n");
    println!("Comparing **sift query** vs **naive grep+cat** for code understanding tasks.\n");

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
        let src_dir = fixture.join("src");
        let tasks_path = fixture.join("tasks.json");

        if !tasks_path.exists() {
            eprintln!("  skipping {}: no tasks.json", name);
            continue;
        }

        // Compute source bytes
        let source_bytes = total_source_bytes(&src_dir);

        // Build sift index
        let index_path = fixture.join(".sift").join("index.bin");
        if index_path.exists() {
            let _ = fs::remove_dir_all(fixture.join(".sift"));
        }

        let start = Instant::now();
        let index_out = Command::new(&sift_bin)
            .arg("index")
            .arg(&fixture)
            .output()
            .expect("sift index failed");
        let index_time = start.elapsed();

        if !index_out.status.success() {
            eprintln!(
                "  FAIL index {}: {}",
                name,
                String::from_utf8_lossy(&index_out.stderr)
            );
            continue;
        }

        let tasks: TaskFile = serde_json::from_str(&fs::read_to_string(&tasks_path).unwrap())
            .expect("invalid tasks.json");

        println!("## {}\n", name);
        println!(
            "  - Source files: {}  |  Total bytes: {}",
            count_files(&src_dir),
            source_bytes
        );
        println!("  - Tasks: {}  |  Index time: {:?}\n", tasks.tasks.len(), index_time);
        println!(
            "| Task | Result | Sift out (bytes) | Naive cost (bytes) | Savings |
|------|--------|-------------------|--------------------|---------|"
        );

        for task in &tasks.tasks {
            let task_start = Instant::now();
            let mut cmd = Command::new(&sift_bin);
            cmd.arg("query").current_dir(&fixture);
            if let Some(limit) = task.limit {
                cmd.arg("--limit").arg(limit.to_string());
            }
            if let Some(offset) = task.offset {
                cmd.arg("--offset").arg(offset.to_string());
            }
            cmd.arg(&task.query);
            let query_out = cmd.output().expect("sift query failed");
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

            // Naive cost: agent reads entire src dir via grep, plus reads matched files
            let naive_cost = estimate_naive_cost(&src_dir, &task.query);
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

fn find_sift_bin(root: &Path) -> PathBuf {
    // Try release first, then debug
    let candidates = [
        root.join("target").join("release").join("sift"),
        root.join("target").join("debug").join("sift"),
    ];
    for c in &candidates {
        if c.exists() {
            return c.clone();
        }
    }
    // Build it
    let status = Command::new("cargo")
        .arg("build")
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
        return false;
    }

    // Try to parse as JSON array
    let val: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => return false,
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
        // At least one of the expected entries must match any result
        for exp in expected_list {
            if arr.iter().any(|entry| entry_matches(entry, exp)) {
                return true;
            }
        }
        eprintln!("    [{}] no expected entry matched any result", task.id);
        return false;
    }

    if let Some(expected_names) = &task.expected_names {
        let names: Vec<&str> = arr
            .iter()
            .filter_map(|v| v.get("callee").or_else(|| v.get("name")))
            .filter_map(|v| v.as_str())
            .collect();
        for n in expected_names {
            if !names.contains(&n.as_str()) {
                eprintln!("    [{}] expected callee/name '{}' not found", task.id, n);
                return false;
            }
        }
        return true;
    }

    if let Some(expected_symbols) = &task.expected_symbols {
        let symbols: Vec<&str> = arr
            .iter()
            .filter_map(|v| v.get("symbols"))
            .filter_map(|v| v.as_array())
            .flatten()
            .filter_map(|v| v.as_str())
            .collect();
        for s in expected_symbols {
            if !symbols.contains(&s.as_str()) {
                eprintln!("    [{}] expected symbol '{}' not found in file listing", task.id, s);
                return false;
            }
        }
        return true;
    }

    if let Some(expected_imports) = &task.expected_imports {
        for exp_imp in expected_imports {
            let matched = arr.iter().any(|entry| {
                let sym = entry.get("symbol").and_then(|v| v.as_str());
                if let Some(expected_sym) = &exp_imp.symbol {
                    if sym != Some(expected_sym.as_str()) {
                        return false;
                    }
                }
                if let Some(expected_resolved) = exp_imp.resolved {
                    let resolved = entry.get("resolved").and_then(|v| v.as_bool());
                    if resolved != Some(expected_resolved) {
                        return false;
                    }
                }
                true
            });
            if !matched {
                eprintln!("    [{}] expected import not matched: {:?}", task.id, exp_imp);
                return false;
            }
        }
        return true;
    }

    true
}

fn matches_entry(arr: &[serde_json::Value], expected: &Expected) -> bool {
    if arr.is_empty() {
        return false;
    }
    // Find any entry that matches the expected fields
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
    if let Some(line) = expected.line {
        if entry.get("line").and_then(|v| v.as_u64()) != Some(line as u64) {
            return false;
        }
    }
    if let Some(caller) = &expected.caller {
        if entry.get("caller").and_then(|v| v.as_str()) != Some(caller.as_str()) {
            return false;
        }
    }
    true
}

fn estimate_naive_cost(src_dir: &Path, query: &str) -> u64 {
    if !src_dir.exists() {
        return 0;
    }

    let symbol = query.split_whitespace().last().unwrap_or(query);
    let mut grep_bytes = 0u64;
    let mut matched_file_bytes = 0u64;

    let mut files: Vec<PathBuf> = Vec::new();
    walk_files(src_dir, &mut |p| files.push(p.to_path_buf()));

    for path in &files {
        let Ok(content) = fs::read_to_string(path) else {
            continue;
        };
        grep_bytes += content.len() as u64;
        if content.contains(symbol) {
            matched_file_bytes += content.len() as u64;
        }
    }

    grep_bytes + matched_file_bytes
}
