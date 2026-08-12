//! KV client on top of the container: a KV table is a JSON section; writes are
//! appended as delta blocks and replayed on read (base + patches in seq order).

use crate::format::{parse_delta_block, slice, Delta};
use crate::reader::EmlBox;
use crate::writer::append_delta;
use serde_json::Value;
use std::path::Path;

pub fn table(b: &EmlBox, table: &str) -> Result<Value, String> {
    let mut out: Value = match b.section(table) {
        Some(s) => serde_json::from_slice(s).map_err(|e| format!("base table {table}: {e}"))?,
        None => Value::Object(serde_json::Map::new()),
    };
    for e in b.tail_entries() {
        let block = slice(&b.mmap, e.off, e.len)?;
        if let Some(delta) = parse_delta_block(block)? {
            if delta.table == table {
                apply(&mut out, &delta)?;
            }
        }
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
        other => Err(format!("unknown delta op: {other}")),
    }
}

pub fn set(path: &Path, table: &str, key: &str, value: Value) -> Result<(u64, String), String> {
    let delta = Delta {
        op: "set".into(),
        table: table.into(),
        key: key.into(),
        value,
        ts: now(),
    };
    append_delta(path, &delta)
}

pub fn del(path: &Path, table: &str, key: &str) -> Result<(u64, String), String> {
    let delta = Delta {
        op: "del".into(),
        table: table.into(),
        key: key.into(),
        value: Value::Null,
        ts: now(),
    };
    append_delta(path, &delta)
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
