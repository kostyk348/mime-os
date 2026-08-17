//! repair: восстановление контейнера после tear-write / краха.
//!
//! Append пишет [блок][tail index][trailer] — при обрыве tail может не
//! совпадать с блоками (или хвост обрезан). repair пересобирает tail index
//! сканом дельта-маркеров от конца базы и усекает файл до последнего целого
//! блока. Цепочки писателей и X-Prev-Hash при этом не трогаются — verify
//! после repair чистый (неполный хвост просто отброшен).

use crate::format::*;
use crate::reader::EmlBox;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

/// Пересобрать tail. Returns (блоков_восстановлено, байт_отброшено).
pub fn repair(path: &Path) -> Result<(usize, u64), String> {
    // 1. начало области блоков: первый дельта-маркер после head index
    //    (tail index в trailer указывает на КОНЕЦ блоков — не годится)
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let (hend, _) = find_blank(&data).ok_or("no envelope")?;
    let headers = parse_headers(&data[..hend]);
    let hoff: u64 = header_get(&headers, "X-Index-Offset")
        .ok_or("no X-Index-Offset")?
        .trim()
        .parse()
        .map_err(|_| "bad index offset")?;
    let hlen: u64 = header_get(&headers, "X-Index-Length")
        .ok_or("no X-Index-Length")?
        .trim()
        .parse()
        .map_err(|_| "bad index length")?;
    let start = (hoff + hlen) as usize;
    let tail_index_off = find_sub(&data[start..], b"X-EMLBox-Delta: v1")
        .map(|r| (start + r) as u64)
        .ok_or("no delta blocks found")?;
    let base_hash = hash_bytes(&data[..tail_index_off as usize]);

    // 2. скан блоков
    let marker = b"X-EMLBox-Delta: v1";
    let mut entries: Vec<(u64, TailEntry, usize, usize)> = Vec::new(); // (off, entry, end)
    let mut pos = tail_index_off as usize;
    while pos + marker.len() <= data.len() {
        let Some(rel) = find_sub(&data[pos..], marker) else { break };
        let block_start = pos + rel;
        let block = &data[block_start..];
        // заголовки до blank
        let Some((hend, hsep)) = find_blank(block) else { break }; // обрезанный блок
        let headers = parse_headers(&block[..hend]);
        let writer = header_get(&headers, "X-Writer-ID").unwrap_or(DEFAULT_WRITER).to_string();
        let seq: u64 = match header_get(&headers, "X-Delta-Seq").and_then(|s| s.trim().parse().ok()) {
            Some(s) => s,
            None => break,
        };
        let body_start = block_start + hend + hsep;
        // конец блока = следующий маркер
        let next = find_sub(&data[body_start..], marker)
            .map(|r| body_start + r)
            .unwrap_or(data.len());
        let block_end = next;
        let block_bytes = &data[block_start..block_end];
        let hash = hash_bytes(block_bytes);
        entries.push((
            block_start as u64,
            TailEntry {
                seq,
                writer,
                off: block_start as u64,
                len: block_bytes.len() as u64,
                hash,
            },
            block_end,
            body_start,
        ));
        pos = block_end;
    }

    if entries.is_empty() {
        // дельт нет — ничего не восстанавливаем
        return Ok((0, 0));
    }
    let last_block_end = entries.last().map(|(_, _, e, _)| *e).unwrap_or(tail_index_off as usize);

    // 3. пересборка tail index + trailer, усечение
    let new_tail = TailIndex { v: 1, entries: entries.iter().map(|(_, e, _, _)| e.clone()).collect() };
    let new_tail_bytes = serde_json::to_vec(&new_tail).map_err(|e| e.to_string())?;
    let new_tail_off = last_block_end as u64;
    let entity = match EmlBox::open(path) {
        Ok(b) => b.entity().unwrap_or_default(),
        Err(_) => "repaired".to_string(),
    };
    let last_hash = entries.last().map(|(_, e, _, _)| e.hash.clone()).unwrap_or_default();
    let trailer = render_trailer(
        &entity,
        new_tail.entries.len() as u64,
        &base_hash,
        &last_hash,
        new_tail_off,
        new_tail_bytes.len() as u64,
    );
    let new_len = new_tail_off + new_tail_bytes.len() as u64 + TRAILER_SIZE as u64;

    let mut f = std::fs::OpenOptions::new().write(true).open(path).map_err(|e| e.to_string())?;
    f.seek(SeekFrom::Start(new_tail_off)).map_err(|e| e.to_string())?;
    f.write_all(&new_tail_bytes).map_err(|e| e.to_string())?;
    f.write_all(&trailer).map_err(|e| e.to_string())?;
    f.set_len(new_len).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;

    let removed = (data.len() as u64).saturating_sub(new_len);
    Ok((entries.len(), removed))
}

fn find_sub(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}
