//! EML-FS: плоский стор контейнеров + header-scan индекс + X-Query +
//! виртуальные директории (System/Directory).
//!
//! Диск = директория с .eml-контейнерами (плоско, без иерархии). При index()
//! каждый контейнер открывается (envelope + index + tail), строится запись:
//! entity / X-EML-Type / Subject / теги (X-Tag заголовки + KV-таблица `tags`)
//! / размер. Директория — тоже контейнер с X-EML-Type: System/Directory,
//! членство: явное X-Contains-ID и/или динамическое X-Query.

use crate::kv;
use crate::query;
use crate::reader::EmlBox;
use std::path::{Path, PathBuf};

pub struct Record {
    pub file: PathBuf,
    pub entity: String,
    pub eml_type: String,
    pub subject: String,
    pub tags: Vec<String>,
    pub size: u64,
    pub headers: Vec<(String, String)>,
}

impl Record {
    fn from_box(b: &EmlBox, file: PathBuf, size: u64) -> Record {
        let entity = b.entity().unwrap_or_default();
        let eml_type = b.header("X-EML-Type").unwrap_or("").to_string();
        let subject = b.header("Subject").unwrap_or("").to_string();
        let mut tags: Vec<String> = b
            .headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("X-Tag"))
            .map(|(_, v)| v.trim().to_string())
            .collect();
        // dynamic tags from KV table `tags` (delta-appended)
        if let Ok(t) = kv::table(b, "tags") {
            if let serde_json::Value::Object(map) = t {
                for (k, v) in &map {
                    if v.as_bool().unwrap_or(false) || !v.is_null() {
                        if !tags.contains(k) {
                            tags.push(k.clone());
                        }
                    }
                }
            }
        }
        tags.sort();
        let mut headers = b.headers.clone();
        // KV-теги видны X-Query как синтетические X-Tag-заголовки
        for t in &tags {
            if !headers.iter().any(|(k, v)| k.eq_ignore_ascii_case("X-Tag") && v == t) {
                headers.push(("X-Tag".to_string(), t.clone()));
            }
        }
        Record {
            file,
            entity,
            eml_type,
            subject,
            tags,
            size,
            headers,
        }
    }
}

/// Scan a store dir and build the index. Returns records and skipped files.
pub fn index(store: &Path) -> Result<(Vec<Record>, Vec<PathBuf>), String> {
    let mut records = Vec::new();
    let mut skipped = Vec::new();
    let rd = std::fs::read_dir(store).map_err(|e| format!("store {store:?}: {e}"))?;
    for entry in rd.flatten() {
        let p = entry.path();
        if p.extension().map(|x| x == "eml").unwrap_or(false) {
            match EmlBox::open(&p) {
                Ok(b) => {
                    let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                    records.push(Record::from_box(&b, p, size));
                }
                Err(_) => skipped.push(p),
            }
        }
    }
    records.sort_by(|a, b| a.entity.cmp(&b.entity));
    Ok((records, skipped))
}

// ---------------------------------------------------------------- X-Query

/// Evaluate an X-Query against the index (AND of clauses).
pub fn eval<'a>(records: &'a [Record], query: &str) -> Result<Vec<&'a Record>, String> {
    query::parse(query)?; // validate syntax once
    Ok(records
        .iter()
        .filter(|r| query::eval_headers(&r.headers, query).unwrap_or(false))
        .collect())
}

// ---------------------------------------------------------------- directories

/// Create a virtual directory container in the store.
pub fn mkdir(
    store: &Path,
    name: &str,
    query: Option<&str>,
    contains: &[String],
) -> Result<PathBuf, String> {
    let entity = format!("{name}@system.local");
    let mut extra = "X-EML-Type: System/Directory\r\n".to_string();
    if let Some(q) = query {
        extra.push_str(&format!("X-Query: {q}\r\n"));
    }
    for c in contains {
        extra.push_str(&format!("X-Contains-ID: <{c}>\r\n"));
    }
    let path = store.join(format!("{name}.eml"));
    crate::writer::build_file_with_headers(&path, &entity, &format!("dir: {name}"), &extra, vec![])?;
    Ok(path)
}

/// Resolve directory membership: explicit X-Contains-ID (existing entities)
/// union dynamic X-Query matches. No duplicates.
pub fn resolve<'a>(records: &'a [Record], dir: &Record) -> Vec<&'a Record> {
    let mut out: Vec<&Record> = Vec::new();
    let mut have: Vec<String> = Vec::new();
    let push = |r: &'a Record, have: &mut Vec<String>, out: &mut Vec<&'a Record>| {
        if !have.contains(&r.entity) {
            have.push(r.entity.clone());
            out.push(r);
        }
    };
    for (k, v) in &dir.headers {
        if k.eq_ignore_ascii_case("X-Contains-ID") {
            let id = v.trim_matches(|c| c == '<' || c == '>' || c == ' ').to_string();
            if let Some(r) = records.iter().find(|r| r.entity == id) {
                push(r, &mut have, &mut out);
            }
        }
    }
    for (k, v) in &dir.headers {
        if k.eq_ignore_ascii_case("X-Query") {
            if let Ok(matches) = eval(records, v) {
                for r in matches {
                    push(r, &mut have, &mut out);
                }
            }
        }
    }
    out
}

/// Tag a container (dynamic, via KV table `tags` -> delta append).
pub fn tag(store: &Path, entity: &str, tag: &str) -> Result<(u64, String), String> {
    let (records, _) = index(store)?;
    let rec = records
        .iter()
        .find(|r| r.entity == entity)
        .ok_or_else(|| format!("entity {entity} not in store"))?;
    kv::set(&rec.file, "tags", tag, serde_json::json!(true))
}
