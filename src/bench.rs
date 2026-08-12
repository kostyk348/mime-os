//! Benchmarks: prove "readable fast, even for system things".
//!   mount  — per-file open cost on N containers (should be O(index), flat vs size)
//!   access — random-access section latency (zero-copy, no body scan)
//!   append — delta append cost vs file size (should be ~constant, base never moves)

use crate::demo;
use crate::kv;
use crate::reader::EmlBox;
use crate::writer::{build_file, Part};
use serde_json::json;
use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

fn ns(t: Instant) -> f64 {
    t.elapsed().as_nanos() as f64
}

/// ns/op formatted with proper unit.
fn fmt_unit(v: f64) -> String {
    if v >= 1e6 {
        format!("{:.2} ms", v / 1e6)
    } else if v >= 1e3 {
        format!("{:.2} us", v / 1e3)
    } else {
        format!("{:.1} ns", v)
    }
}

/// Create n small containers in `dir`, then open all of them. Reports total and per-file.
pub fn mount_bench(dir: &Path, n: usize) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let t_create = Instant::now();
    for i in 0..n {
        let p = dir.join(format!("f{i:05}.eml"));
        build_file(
            &p,
            &format!("bench{i}"),
            &format!("bench file {i}"),
            vec![
                Part::raw("meta", "application/json", "meta.json", format!(r#"{{"i":{i}}}"#).into_bytes()),
                Part::raw("view", "text/html", "view.html", format!("<html><b>{i}</b></html>").into_bytes()),
            ],
        )?;
    }
    let create_us = ns(t_create) / n as f64;

    let t_mount = Instant::now();
    let mut total = 0usize;
    for i in 0..n {
        let b = EmlBox::open(&dir.join(format!("f{i:05}.eml")))?;
        total += b.sections.len();
    }
    let mount_us = ns(t_mount) / n as f64;

    Ok(format!(
        "mount_bench: n={n} files, create={}/file, open={}/file, sections seen={total}",
        fmt_unit(create_us),
        fmt_unit(mount_us),
    ))
}

/// Random access: open a container with a big binary section; repeatedly fetch it.
pub fn access_bench(path: &Path, id: &str, iters: usize) -> Result<String, String> {
    let b = EmlBox::open(path)?;
    let len = b.section(id).map(|s| s.len()).ok_or("section not found")?;
    let t = Instant::now();
    let mut acc = 0usize;
    for _ in 0..iters {
        let s = b.section(id).ok_or("section vanished")?;
        acc += black_box(s.len());
    }
    let per = ns(t) / iters as f64;
    Ok(format!(
        "access_bench: section '{id}' len={len}, {iters} fetches, {}/fetch (acc={acc})",
        fmt_unit(per)
    ))
}

/// Append: N deltas to a container; report per-append cost over time.
pub fn append_bench(path: &Path, n: usize) -> Result<String, String> {
    if !path.exists() {
        build_file(
            path,
            "bench",
            "append bench",
            vec![Part::raw("users", "application/json", "users.json", br#"{}"#.to_vec())],
        )?;
    }
    let mut samples = Vec::new();
    for i in 0..n {
        let t = Instant::now();
        kv::set(path, "users", &format!("k{i}"), json!({"i": i, "pad": "x".repeat(32)}))?;
        samples.push(ns(t));
    }
    let head = samples[0];
    let mid = samples[n / 2];
    let tail = samples[n - 1];
    let sum: f64 = samples.iter().sum();
    let avg = sum / n as f64;
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Ok(format!(
        "append_bench: n={n} deltas, first={}, mid={}, last={}, avg={}, file now {size} B",
        fmt_unit(head),
        fmt_unit(mid),
        fmt_unit(tail),
        fmt_unit(avg),
    ))
}

/// Full run: creates demo + big file, reports all three numbers.
pub fn run_all(dir: &Path) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let demo_path = dir.join("game.eml");
    demo::build_demo(&demo_path, false)?;

    let big = dir.join("big.eml");
    demo::build_demo(&big, true)?;

    let m = mount_bench(&dir.join("mount"), 2000)?;
    let a = access_bench(&demo_path, "sprites", 1_000_000)?;
    let ap = append_bench(&dir.join("append.eml"), 200)?;
    Ok(format!("{m}\n{a}\n{ap}"))
}
