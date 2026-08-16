//! Сетевая фаза: delta-sync между устройствами.
//!
//! Транспорт — та же .eml-шина (spool-директория), что и EML-IPC: локальный и
//! сетевой обмен неотличимы (конверт один, транспорт заменяем на SMTP/QUIC).
//!
//! Сообщение в шине:
//!
//!   From: <entity@device.local>
//!   To: <target-entity>@system.local
//!   X-EMLBox-Sync: v1
//!   X-Writer-ID: devA
//!   Content-Type: application/x-emlbox-delta
//!
//!   <raw delta block bytes, verbatim>
//!
//! pull применяет только применимые блоки (нет пропущенных предшественников);
//! блоки вне порядка остаются в шине pending до прихода недостающего.

use crate::format::{find_blank, header_get, parse_headers, slice};
use crate::reader::EmlBox;
use crate::writer::append_block_w;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Все блоки писателя с seq > since. Returns (seq, raw_bytes) по возрастанию seq.
pub fn export(container: &Path, writer: &str, since: u64) -> Result<Vec<(u64, Vec<u8>)>, String> {
    let b = EmlBox::open(container)?;
    let mut out = Vec::new();
    for e in b.tail_entries() {
        if e.writer == writer && e.seq > since {
            let block = slice(&b.mmap, e.off, e.len)?.to_vec();
            out.push((e.seq, block));
        }
    }
    out.sort_by_key(|(seq, _)| *seq);
    Ok(out)
}

/// Головы цепочек писателей: (writer, last_seq, last_hash).
pub fn heads(container: &Path) -> Result<Vec<(String, u64, String)>, String> {
    let b = EmlBox::open(container)?;
    let mut map: std::collections::HashMap<String, (u64, String)> = std::collections::HashMap::new();
    for e in b.tail_entries() {
        match map.get_mut(&e.writer) {
            Some(cur) if e.seq > cur.0 => *cur = (e.seq, e.hash.clone()),
            Some(_) => {}
            None => {
                map.insert(e.writer.clone(), (e.seq, e.hash.clone()));
            }
        }
    }
    let mut out: Vec<(String, u64, String)> =
        map.into_iter().map(|(w, (s, h))| (w, s, h)).collect();
    out.sort();
    Ok(out)
}

/// Применить чужой дельта-блок дословно (writer/seq берутся из самого блока).
pub fn apply_block(container: &Path, block: &[u8]) -> Result<(String, u64), String> {
    let (end, _) = find_blank(block).ok_or("sync block: no headers")?;
    let h = parse_headers(&block[..end]);
    let writer = header_get(&h, "X-Writer-ID").unwrap_or("local").to_string();
    let seq: u64 = header_get(&h, "X-Delta-Seq").ok_or("no seq")?.trim().parse().map_err(|_| "bad seq")?;
    append_block_w(container, &writer, block)?;
    Ok((writer, seq))
}

fn msg_bytes(entity: &str, to: &str, writer: &str, block: &[u8]) -> Vec<u8> {
    let mut buf = format!(
        "From: <{entity}@system.local>\r\nTo: <{to}>\r\nX-EMLBox-Sync: v1\r\n\
         X-Writer-ID: {writer}\r\nContent-Type: application/x-emlbox-delta\r\n\r\n"
    )
    .into_bytes();
    buf.extend_from_slice(block);
    buf
}

/// Экспортировать блоки писателя (seq > since) в шину как sync-сообщения.
pub fn push(container: &Path, writer: &str, bus: &Path, to: &str, since: u64) -> Result<usize, String> {
    std::fs::create_dir_all(bus).map_err(|e| e.to_string())?;
    let b = EmlBox::open(container)?;
    let entity = b.entity().ok_or("no entity")?;
    let blocks = export(container, writer, since)?;
    let mut n = 0;
    for (seq, block) in &blocks {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let name = format!("sync_{nanos}_{writer}_{seq}.msg.eml");
        let path = bus.join(name);
        std::fs::write(&path, msg_bytes(&entity, to, writer, block)).map_err(|e| e.to_string())?;
        n += 1;
    }
    Ok(n)
}

/// Применить все применимые sync-сообщения из шины. Returns (applied, pending).
///
/// Цикл до стабилизации: блок, пришедший раньше своего предшественника,
/// отклоняется на этом проходе и применяется на следующем, когда предшественник
/// доставлен. Идемпотентность: повторно пришедший (writer, seq) — .done, не pending.
pub fn pull(container: &Path, bus: &Path) -> Result<(usize, usize), String> {
    let entity = {
        let b = EmlBox::open(container)?;
        b.entity().ok_or("no entity")?.to_string()
    };
    let rd = std::fs::read_dir(bus).map_err(|e| e.to_string())?;
    let mut files: Vec<PathBuf> = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if name.ends_with(".msg.eml") && !name.contains("ignore") {
            files.push(p);
        }
    }
    files.sort();

    let mut applied = 0usize;
    let mut remaining = files;
    let mut changed = true;
    while changed && !remaining.is_empty() {
        changed = false;
        let b = EmlBox::open(container)?;
        let mut next_round: Vec<PathBuf> = Vec::new();
        for p in &remaining {
            let data = match std::fs::read(p) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let (end, sep) = match find_blank(&data) {
                Some(x) => x,
                None => continue,
            };
            let h = parse_headers(&data[..end]);
            if header_get(&h, "X-EMLBox-Sync").is_none() {
                continue; // не sync-сообщение
            }
            let to = header_get(&h, "To").unwrap_or("").trim_matches(|c| c == '<' || c == '>' || c == ' ');
            if to != entity && to != "*" {
                continue; // не нам
            }
            let block = &data[end + sep..];
            let (wend, _) = match find_blank(block) {
                Some(x) => x,
                None => continue,
            };
            let bh = parse_headers(&block[..wend]);
            let writer = header_get(&bh, "X-Writer-ID").unwrap_or("local").to_string();
            let seq: u64 = header_get(&bh, "X-Delta-Seq")
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            // идемпотентность: блок уже применён (та же writer#seq)
            let exists = b.tail_entries().iter().any(|e| e.writer == writer && e.seq == seq);
            if exists {
                applied += 1;
                changed = true;
                let done = PathBuf::from(format!("{}.done", p.display()));
                let _ = std::fs::rename(p, &done);
                continue;
            }
            match append_block_w(container, &writer, block) {
                Ok(_) => {
                    applied += 1;
                    changed = true;
                    let done = PathBuf::from(format!("{}.done", p.display()));
                    let _ = std::fs::rename(p, &done);
                }
                Err(_) => next_round.push(p.clone()), // ждёт предшественника
            }
        }
        remaining = next_round;
    }
    let pending = remaining.len();
    Ok((applied, pending))
}
