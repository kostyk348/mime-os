//! KV client on top of the container: a KV table is a JSON section; writes are
//! appended as delta blocks and replayed on read (base + patches).
//!
//! Multi-writer replay: внутри каждого писателя — строго по seq (причинность),
//! между писателями — LWW по (ts, writer). Стабильная сортировка сохраняет
//! порядок внутри писателя при равных (ts, writer) → детерминизм на всех
//! устройствах.

use crate::format::{parse_delta_block, slice, Delta, DEFAULT_WRITER};
use crate::reader::EmlBox;
use crate::writer::{append_delta, append_delta_w};
use serde_json::{json, Value};
use std::path::Path;

pub fn table(b: &EmlBox, table: &str) -> Result<Value, String> {
    let has_section = b.sections.iter().any(|s| s.id == table);
    let mut out: Value = if has_section {
        let raw = b.section_checked(table)?; // ошибка декодирования видна
        serde_json::from_slice(&raw).map_err(|e| format!("base table {table}: {e}"))?
    } else {
        Value::Object(serde_json::Map::new())
    };
    // (ts, writer, seq, delta): stable sort по (ts, writer) → внутри писателя
    // при равных (ts, writer) сохраняется порядок вставки, а он по построению
    // (append_block_w валидирует seq) совпадает с порядком по seq.
    let mut items: Vec<(u64, String, u64, Delta)> = Vec::new();
    for e in b.tail_entries() {
        let block = slice(&b.mmap, e.off, e.len)?;
        if let Some(delta) = parse_delta_block(block)? {
            if delta.table == table {
                items.push((delta.ts, e.writer.clone(), e.seq, delta));
            }
        }
    }
    items.sort_by(|a, c| (a.0, &a.1).cmp(&(c.0, &c.1)));
    for (_, _, _, delta) in items {
        apply(&mut out, &delta)?;
    }
    Ok(out)
}

pub fn get(b: &EmlBox, tbl: &str, key: &str) -> Result<Option<Value>, String> {
    let t = table(b, tbl)?;
    Ok(t.get(key).cloned())
}

fn apply(out: &mut Value, delta: &Delta) -> Result<(), String> {
    match delta.op.as_str() {
        "set" => match out {
            Value::Object(map) => {
                map.insert(delta.key.clone(), delta.value.clone());
                Ok(())
            }
            _ => Err(format!("table is not an object (op=set {})", delta.table)),
        },
        "del" => match out {
            Value::Object(map) => {
                map.remove(&delta.key);
                Ok(())
            }
            _ => Err(format!("table is not an object (op=del {})", delta.table)),
        },
        "add" => {
            // RGA-вставка в список: элементы {"id","v"}, id = "writer#seq"
            let arr = out
                .as_object_mut()
                .ok_or(format!("table is not an object (op=add {})", delta.table))?
                .entry(delta.key.clone())
                .or_insert_with(|| Value::Array(Vec::new()));
            let Value::Array(arr) = arr else {
                return Err(format!("key '{}' is not a list", delta.key));
            };
            let pos = match &delta.after {
                Some(a) => arr
                    .iter()
                    .position(|e| e.get("id").and_then(|i| i.as_str()) == Some(a.as_str()))
                    .map(|p| p + 1)
                    .unwrap_or(arr.len()),
                None => arr.len(),
            };
            arr.insert(pos, json!({ "id": delta.id.clone().unwrap_or_default(), "v": delta.value.clone() }));
            Ok(())
        }
        other => Err(format!("unknown delta op: {other}")),
    }
}

pub fn set(path: &Path, table: &str, key: &str, value: Value) -> Result<(u64, String), String> {
    set_w(path, DEFAULT_WRITER, table, key, value)
}

/// Write with an explicit writer id (для сети: каждое устройство — свой writer).
pub fn set_w(path: &Path, writer: &str, table: &str, key: &str, value: Value) -> Result<(u64, String), String> {
    let delta = Delta {
        op: "set".into(),
        table: table.into(),
        key: key.into(),
        value,
        ts: now(),
        id: None,
        after: None,
    };
    append_delta_w(path, writer, &delta)
}

pub fn del(path: &Path, table: &str, key: &str) -> Result<(u64, String), String> {
    let delta = Delta {
        op: "del".into(),
        table: table.into(),
        key: key.into(),
        value: Value::Null,
        ts: now(),
        id: None,
        after: None,
    };
    append_delta(path, &delta)
}

/// RGA-добавление в список: value вставляется после элемента `after`
/// (None = в конец). id элемента = "writer#seq" — уникален, порядок слияния
/// детерминирован (LWW по ts,writer), реплики сходятся.
pub fn add(
    path: &Path,
    writer: &str,
    table: &str,
    key: &str,
    value: Value,
    after: Option<String>,
) -> Result<(u64, String), String> {
    let seq = crate::writer::next_seq(path, writer)?;
    let delta = Delta {
        op: "add".into(),
        table: table.into(),
        key: key.into(),
        value,
        ts: now(),
        id: Some(format!("{writer}#{seq}")),
        after,
    };
    append_delta_w(path, writer, &delta)
}

/// Значения списка (без id-обёртки).
pub fn list(b: &EmlBox, tbl: &str, key: &str) -> Result<Vec<Value>, String> {
    let t = table(b, tbl)?;
    match t.get(key) {
        Some(Value::Array(arr)) => Ok(arr.iter().map(|e| e.get("v").cloned().unwrap_or(e.clone())).collect()),
        Some(_) => Err(format!("key '{key}' is not a list")),
        None => Ok(Vec::new()),
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
