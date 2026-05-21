//! Incremental re-index benchmark.
//!
//! Measures how much faster incremental re-index is compared to full re-index:
//!   1. Full index of a fixture
//!   2. Re-index with no source changes (should be near-instant, just mtime check)
//!   3. Touch one file → re-index (should re-parse only that file)
//!   4. Touch all files → re-index (worst case: same as full index)
//!   5. Print comparison table

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn main() {
    let root = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string()),
    );
    let fixtures_dir = root.join("bench-fixtures");

    let sift_bin = find_sift_bin(&root);

    println!("# Incremental Re-index Benchmark\n");
    println!(
        "Measures time savings of incremental vs full re-index across different change scenarios.\n"
    );

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

        if !src_dir.exists() {
            continue;
        }

        // Count source files
        let source_count = count_files(&src_dir);
        if source_count == 0 {
            continue;
        }

        println!("## {}\n", name);

        // Phase 1: Full index (fresh)
        clean_index(&fixture);
        let full_time = measure_index(&sift_bin, &fixture, &[]);
        println!("| Scenario | Time | vs Full | Notes |");
        println!("|----------|------|---------|-------|");
        println!(
            "| Full index | {:.3}s | 1.00x | {} source files |",
            full_time.as_secs_f64(),
            source_count,
        );

        // Phase 2: Re-index with no changes
        let idle_time = measure_index(&sift_bin, &fixture, &[]);
        println!(
            "| Re-index (no changes) | {:.3}s | {:.1}x | mtime comparison only |",
            idle_time.as_secs_f64(),
            full_time.as_secs_f64().max(0.001) / idle_time.as_secs_f64().max(0.0001),
        );

        // Phase 3: Touch one file
        let files: Vec<PathBuf> = collect_files(&src_dir);
        if let Some(first) = files.first() {
            touch_file(first);
            let one_change_time = measure_index(&sift_bin, &fixture, &[]);
            println!(
                "| Re-index (1 file changed) | {:.3}s | {:.1}x | re-parses 1 file only |",
                one_change_time.as_secs_f64(),
                full_time.as_secs_f64().max(0.001) / one_change_time.as_secs_f64().max(0.0001),
            );
        }

        // Phase 4: Touch all files (worst case)
        for f in &files {
            touch_file(f);
        }
        let all_change_time = measure_index(&sift_bin, &fixture, &[]);
        println!(
            "| Re-index (all files changed) | {:.3}s | {:.1}x | same as full re-parse |",
            all_change_time.as_secs_f64(),
            full_time.as_secs_f64().max(0.001) / all_change_time.as_secs_f64().max(0.0001),
        );

        println!();
    }

    println!("## Summary\n");
    println!("Incremental re-index avoids re-parsing unchanged files by comparing stored mtimes.");
    println!("The \"no changes\" case should be near-instant (just mtree comparison + save).");
    println!("The \"1 file changed\" case should be faster than full index for large codebases.");
}

fn find_sift_bin(root: &Path) -> PathBuf {
    let candidates = [
        root.join("target").join("release").join("sift"),
        root.join("target").join("debug").join("sift"),
    ];
    for c in &candidates {
        if c.exists() {
            return c.clone();
        }
    }
    let status = Command::new("cargo")
        .arg("build")
        .current_dir(root)
        .status()
        .expect("cargo build failed");
    assert!(status.success(), "cargo build failed");
    root.join("target").join("debug").join("sift")
}

fn measure_index(sift_bin: &Path, fixture: &Path, args: &[&str]) -> Duration {
    let start = Instant::now();
    let mut cmd = Command::new(sift_bin);
    cmd.arg("index").args(args).arg(fixture);
    let out = cmd.output().expect("sift index failed");
    let elapsed = start.elapsed();
    if !out.status.success() {
        eprintln!(
            "  FAIL index: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    elapsed
}

fn clean_index(fixture: &Path) {
    let dir = fixture.join(".sift");
    if dir.exists() {
        let _ = fs::remove_dir_all(&dir);
    }
}

fn touch_file(path: &Path) {
    // Update mtime by rewriting content
    if let Ok(content) = fs::read_to_string(path) {
        let _ = fs::write(path, &content);
    }
}

fn collect_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    walk_files(dir, &mut |p| files.push(p.to_path_buf()));
    files
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
