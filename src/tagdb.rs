//! eml-tag: плоская теговая БД (Folderless Tag-DB).
//!
//! Каждая запись — атомарный .eml-файл в одной папке:
//!
//!   X-Record-ID: rec_90412
//!   X-Tag: sensor_data
//!   X-Tag: telemetry
//!   X-Device-ID: node_01
//!   X-Timestamp: 1786528800
//!   Content-Type: application/json
//!
//!   {"temp": 24.5, "voltage": 3.3}
//!
//! * Поиск = header-only scan: mmap + парсинг envelope, тело никогда не читается.
//! * Corruption-proof: запись идёт через tmp+rename (атомарно); сбой убивает
//!   максимум одну запись, остальные читаются. Повреждённые файлы пропускаются.

use crate::format::{find_blank, parse_headers};
use crate::query;
use serde_json::Value;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Max bytes read per record during header scan. Headers of a record are small
/// by design (манифест: «первые 512 байт»); body pages are never touched.
pub const HEADER_SCAN_LIMIT: usize = 4096;

pub fn record_bytes(
    id: &str,
    tags: &[String],
    device: &str,
    ts: u64,
    body: &Value,
) -> Result<Vec<u8>, String> {
    let mut hdr = format!("X-EMLBox-Record: v1\r\nX-Record-ID: {id}\r\n");
    for t in tags {
        hdr.push_str(&format!("X-Tag: {t}\r\n"));
    }
    if !device.is_empty() {
        hdr.push_str(&format!("X-Device-ID: {device}\r\n"));
    }
    hdr.push_str(&format!("X-Timestamp: {ts}\r\nContent-Type: application/json\r\n\r\n"));
    let mut buf = hdr.into_bytes();
    buf.extend_from_slice(serde_json::to_vec(body).map_err(|e| e.to_string())?.as_slice());
    Ok(buf)
}

/// Read ONLY the envelope (up to the first blank line), bounded read.
fn read_headers(p: &Path) -> Result<Vec<(String, String)>, String> {
    let mut f = File::open(p).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; HEADER_SCAN_LIMIT];
    let n = f.read(&mut buf).map_err(|e| e.to_string())?;
    let data = &buf[..n];
    let (end, _) = find_blank(data).ok_or("no blank line within header scan limit")?;
    Ok(parse_headers(&data[..end]))
}

/// Atomic insert: write tmp + rename. A power cut can never leave a torn record.
pub fn insert(db: &Path, id: &str, tags: &[String], device: &str, ts: u64, body: &Value) -> Result<PathBuf, String> {
    std::fs::create_dir_all(db).map_err(|e| e.to_string())?;
    let buf = record_bytes(id, tags, device, ts, body)?;
    let tmp = db.join(format!("{id}.eml.tmp"));
    let final_path = db.join(format!("{id}.eml"));
    std::fs::write(&tmp, &buf).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &final_path).map_err(|e| e.to_string())?;
    Ok(final_path)
}

/// Header-only scan: bounded read of the first bytes, body never touched.
/// Corrupted files are skipped (returned separately).
pub fn scan(db: &Path) -> Result<(Vec<(PathBuf, Vec<(String, String)>)>, usize), String> {
    let mut out = Vec::new();
    let mut corrupt = 0usize;
    let rd = std::fs::read_dir(db).map_err(|e| e.to_string())?;
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().map(|x| x == "eml").unwrap_or(false) {
            match read_headers(&p) {
                Ok(h) => out.push((p, h)),
                Err(_) => corrupt += 1,
            }
        }
    }
    Ok((out, corrupt))
}

/// Query over header-only scan. Returns (matches, corrupt_count).
pub fn query(db: &Path, q: &str) -> Result<(Vec<(PathBuf, Vec<(String, String)>)>, usize), String> {
    let (records, corrupt) = scan(db)?;
    let mut out = Vec::new();
    for (p, h) in &records {
        if query::eval_headers(h, q)? {
            out.push((p.clone(), h.clone()));
        }
    }
    Ok((out, corrupt))
}

/// Count records and total header-bytes scanned (informational).
pub fn stats(db: &Path) -> Result<(usize, u64, usize), String> {
    let (records, corrupt) = scan(db)?;
    let mut hbytes = 0u64;
    for (p, _) in &records {
        hbytes += std::fs::metadata(p).map(|m| m.len()).unwrap_or(0).min(512);
    }
    Ok((records.len(), hbytes, corrupt))
}

// ---------------------------------------------------------------- benchmarks

fn fmt_unit(v: f64) -> String {
    if v >= 1e6 {
        format!("{:.2} ms", v / 1e6)
    } else if v >= 1e3 {
        format!("{:.2} us", v / 1e3)
    } else {
        format!("{:.1} ns", v)
    }
}

/// eml-tag vs обычный FS (плоские JSON-файлы, grep-семантика) vs single-file
/// контейнер. body_kb — размер payload на запись (телеметрия/логи).
pub fn bench(db: &Path, n: usize, body_kb: usize) -> Result<String, String> {
    std::fs::create_dir_all(db).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(db);
    std::fs::create_dir_all(db).map_err(|e| e.to_string())?;

    let payload: String = "x".repeat(body_kb.saturating_mul(1024));
    let body = |i: usize| serde_json::json!({"samples": payload, "temp": 24.5, "seq": i});
    let hit_every = 10usize;

    // ---- A: eml-tag insert
    let t = std::time::Instant::now();
    for i in 0..n {
        let tags = vec![
            "sensor_data".to_string(),
            if i % hit_every == 0 { "telemetry".to_string() } else { "status_ok".to_string() },
        ];
        insert(db, &format!("rec_{i:06}"), &tags, "node_01", 1786528800 + i as u64, &body(i))?;
    }
    let tagdb_insert = t.elapsed().as_nanos() as f64 / n as f64;

    // ---- B: обычный FS — плоские JSON-файлы с тегами внутри тела
    let flat = db.join("flat");
    std::fs::create_dir_all(&flat).map_err(|e| e.to_string())?;
    let t = std::time::Instant::now();
    for i in 0..n {
        let rec = serde_json::json!({
            "id": format!("rec_{i:06}"),
            "tags": if i % hit_every == 0 { ["telemetry"] } else { ["status_ok"] },
            "samples": payload, "temp": 24.5
        });
        std::fs::write(flat.join(format!("rec_{i:06}.json")), serde_json::to_vec(&rec).unwrap()).map_err(|e| e.to_string())?;
    }
    let flat_insert = t.elapsed().as_nanos() as f64 / n as f64;

    // ---- C: контейнер (single-file DB): все записи в base-секции
    let mut all = serde_json::Map::new();
    for i in 0..n {
        all.insert(
            format!("rec_{i:06}"),
            serde_json::json!({"tags": if i % hit_every == 0 {["telemetry"]} else {["status_ok"]}, "samples": payload, "temp": 24.5}),
        );
    }
    let t = std::time::Instant::now();
    crate::writer::build_file(
        &db.join("single.eml"),
        "db_single@system.local",
        "DB: all records",
        vec![crate::writer::Part::raw("recs", "application/json", "recs.json", serde_json::to_vec(&all).unwrap())],
    )?;
    let single_build = t.elapsed().as_nanos() as f64;

    // ---- query: telemetry (n/10 matches)
    let t = std::time::Instant::now();
    let (matches, corrupt) = query(db, "X-Tag == \"telemetry\"")?;
    let tagdb_query = t.elapsed().as_nanos() as f64 / 1e6;
    let tagdb_hit = matches.len();
    let mut tagdb_read: u64 = 0;
    for e in std::fs::read_dir(db).map_err(|e| e.to_string())? {
        let p = e.map_err(|e| e.to_string())?.path();
        if p.extension().map(|x| x == "eml").unwrap_or(false) {
            let sz = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            tagdb_read += sz.min(HEADER_SCAN_LIMIT as u64);
        }
    }

    let t = std::time::Instant::now();
    let mut flat_hit = 0usize;
    let mut flat_read: u64 = 0;
    for e in std::fs::read_dir(&flat).map_err(|e| e.to_string())? {
        let p = e.map_err(|e| e.to_string())?.path();
        flat_read += std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        if let Ok(s) = std::fs::read_to_string(&p) {
            if s.contains("\"telemetry\"") {
                flat_hit += 1;
            }
        }
    }
    let flat_query = t.elapsed().as_nanos() as f64 / 1e6;

    let t = std::time::Instant::now();
    let b = crate::reader::EmlBox::open(&db.join("single.eml"))?;
    let recs: Value = serde_json::from_slice(b.section("recs").unwrap()).map_err(|e| e.to_string())?;
    let mut single_hit = 0usize;
    if let Value::Object(map) = &recs {
        for v in map.values() {
            if v.get("tags").map(|x| x == &serde_json::json!(["telemetry"])).unwrap_or(false) {
                single_hit += 1;
            }
        }
    }
    let single_query = t.elapsed().as_nanos() as f64 / 1e6;

    Ok(format!(
        "tagdb vs flat-FS vs single-file container (n={n}, body={body_kb} KiB, query 'X-Tag == \"telemetry\"', hits={tagdb_hit}/{flat_hit}/{single_hit}):\n\
         insert : tagdb={}  flat-json={}\n\
         query  : tagdb(header-scan)={:.3} ms ({:.1} MiB read)  flat(full-read)={:.3} ms ({:.1} MiB read)  container(base-json)={:.3} ms\n\
         corrupt skipped: {corrupt}; container build: {}",
        fmt_unit(tagdb_insert),
        fmt_unit(flat_insert),
        tagdb_query,
        tagdb_read as f64 / 1e6,
        flat_query,
        flat_read as f64 / 1e6,
        single_query,
        fmt_unit(single_build),
    ))
}
