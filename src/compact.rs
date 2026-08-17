//! Compaction: слияние дельт в base.
//!
//! Новый контейнер = те же секции (KV-таблицы консолидированы дельтами) +
//! новые таблицы из дельт, пустой дельта-лог. Все цепочки писателей
//! сбрасываются к новому base — это точка схождения после синков.
//! X-Encoding секций сохраняется (данные пере-кодируются тем же enc).

use crate::format::{parse_delta_block, slice};
use crate::kv;
use crate::reader::EmlBox;
use crate::writer::{build_file_with_headers, Part};
use std::collections::BTreeSet;
use std::path::Path;

/// Сконсолидировать контейнер в новый файл. Returns (секций, дельт_слито).
pub fn compact(src: &Path, out: &Path) -> Result<(usize, usize), String> {
    let b = EmlBox::open(src)?;
    let entity = b.entity().ok_or("container has no X-Entity-ID")?;
    let subject = b.header("Subject").unwrap_or("").to_string();

    // сохранить пользовательские заголовки (без технических)
    let mut extra = String::new();
    for (k, v) in &b.headers {
        if k.starts_with("X-")
            && !matches!(
                k.as_str(),
                "X-Index-Offset" | "X-Index-Length" | "X-EML-Version" | "X-Entity-ID" | "X-EML-Type"
            )
        {
            extra.push_str(&format!("{k}: {v}\r\n"));
        }
    }

    // таблицы, упомянутые в дельтах
    let mut delta_tables: BTreeSet<String> = BTreeSet::new();
    for e in b.tail_entries() {
        let block = slice(&b.mmap, e.off, e.len)?;
        if let Some(d) = parse_delta_block(block)? {
            delta_tables.insert(d.table);
        }
    }

    let mut parts = Vec::new();
    let n_deltas = b.tail_entries().len();
    for s in &b.sections {
        // KV-таблица: консолидировать дельтами
        if s.ct.contains("json") || delta_tables.contains(&s.id) {
            let consolidated = kv::table(&b, &s.id)?;
            let data = serde_json::to_vec(&consolidated).map_err(|e| e.to_string())?;
            parts.push(Part {
                id: s.id.clone(),
                ct: s.ct.clone(),
                name: s.name.clone(),
                enc: s.enc.clone(),
                data,
            });
        } else {
            // прочие секции: декодированные данные как есть, enc сохраняется
            let data = b.section(&s.id).ok_or_else(|| format!("section {} read failed", s.id))?;
            parts.push(Part {
                id: s.id.clone(),
                ct: s.ct.clone(),
                name: s.name.clone(),
                enc: s.enc.clone(),
                data,
            });
        }
    }
    // новые таблицы, появившиеся в дельтах (не было в base)
    for t in &delta_tables {
        if !b.sections.iter().any(|s| &s.id == t) {
            let consolidated = kv::table(&b, t)?;
            let data = serde_json::to_vec(&consolidated).map_err(|e| e.to_string())?;
            parts.push(Part {
                id: t.clone(),
                ct: "application/json".into(),
                name: format!("{t}.json"),
                enc: "raw".into(),
                data,
            });
        }
    }

    build_file_with_headers(out, &entity, &subject, &extra, parts.clone())?;
    Ok((parts.len(), n_deltas))
}
