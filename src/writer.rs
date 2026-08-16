//! Writer: single-pass container build (create) + append-only delta log.
//!
//! create: assembles envelope + multipart + head index + empty tail index + trailer
//! into one byte buffer with exact absolute offsets (index-driven, no rescan).
//!
//! append: writes a delta block *before* the tail index, then rewrites only
//! [tail index .. EOF] — the base prefix never moves.
//!
//! Сетевой режим (multi-writer): каждый дельта-блок несёт X-Writer-ID,
//! X-Delta-Seq (номер внутри писателя) и X-Prev-Hash (предыдущий блок ТОГО ЖЕ
//! писателя, или base_hash для seq=1). Чужие блоки применяются дословно
//! (append_block_w) — байты не меняются, hash-chain писателя сохраняется,
//! все цепочки сходятся на общем X-Base-Hash.

use crate::format::*;
use crate::reader::EmlBox;
use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

#[derive(Clone)]
pub struct Part {
    pub id: String,
    pub ct: String,
    pub name: String,
    pub enc: String,
    pub data: Vec<u8>,
}

impl Part {
    pub fn raw(id: &str, ct: &str, name: &str, data: Vec<u8>) -> Self {
        Part {
            id: id.to_string(),
            ct: ct.to_string(),
            name: name.to_string(),
            enc: ENC_RAW.to_string(),
            data,
        }
    }
}

/// Build a new container. Returns the absolute offset of the head index payload.
pub fn build_file(path: &Path, entity: &str, subject: &str, parts: Vec<Part>) -> Result<(), String> {
    build_file_with_headers(path, entity, subject, "X-EML-Type: Application/Unified\r\n", parts)
}

/// Build with extra envelope header lines (e.g. X-EML-Type: System/Directory,
/// X-Query, X-Contains-ID, X-Tag). Extra lines go before X-EML-Version.
pub fn build_file_with_headers(
    path: &Path,
    entity: &str,
    subject: &str,
    extra_headers: &str,
    parts: Vec<Part>,
) -> Result<(), String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let boundary = format!("EMLBOX_v1_{nanos:016x}");

    // Envelope prefix is deterministic; index lines are fixed 20-char fields.
    let pre = format!(
        "From: <{entity}@system.local>\r\nTo: <kernel@system.local>\r\nSubject: {subject}\r\n\
         X-Entity-ID: {entity}\r\n{extra_headers}X-EML-Version: {VERSION}\r\n"
    );
    // "X-Index-Offset: " = 16 chars + 20 + \r\n(2) = 38; two lines = 76
    let head_fixed = pre.len() + 76 + format!("Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\r\n").len();

    // ---- assemble body (data sections + head index), tracking absolute offsets
    let mut body = Vec::new();
    let mut sections = Vec::new();
    let mut offs = 0usize; // relative to body start

    for p in &parts {
        // X-Encoding: raw|deflate|aes — секция хранится закодированной
        let enc_data = crate::encoding::encode(&p.enc, &p.data).map_err(|e| format!("{}: {e}", p.id))?;
        let ph = format!(
            "--{boundary}\r\nContent-Type: {}; name=\"{}\"\r\nContent-ID: <{}>\r\nX-Encoding: {}\r\n\r\n",
            p.ct, p.name, p.id, p.enc
        );
        sections.push(SectionInfo {
            id: p.id.clone(),
            ct: p.ct.clone(),
            name: p.name.clone(),
            off: (head_fixed + offs + ph.len()) as u64,
            len: enc_data.len() as u64,
            enc: p.enc.clone(),
        });
        body.extend_from_slice(ph.as_bytes());
        body.extend_from_slice(&enc_data);
        offs += ph.len() + enc_data.len();
    }

    let index = HeadIndex { v: 1, sections };
    let idx_bytes = serde_json::to_vec(&index).map_err(|e| e.to_string())?;
    let idx_ph = format!("--{boundary}\r\nContent-Type: {INDEX_CT}\r\nContent-ID: <eml-index>\r\nX-Encoding: raw\r\n\r\n");
    let idx_off = (head_fixed + offs + idx_ph.len()) as u64;
    let idx_len = idx_bytes.len() as u64;
    body.extend_from_slice(idx_ph.as_bytes());
    body.extend_from_slice(&idx_bytes);
    offs += idx_ph.len() + idx_bytes.len();

    let close = format!("--{boundary}--\r\n");
    body.extend_from_slice(close.as_bytes());
    offs += close.len();

    // ---- tail index (empty at creation) + trailer
    let tail_bytes = serde_json::to_vec(&TailIndex { v: 1, entries: vec![] }).map_err(|e| e.to_string())?;
    let tail_off = (head_fixed + offs) as u64;

    let env = format!(
        "{pre}X-Index-Offset: {idx_off:020}\r\nX-Index-Length: {idx_len:020}\r\n\
         Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\r\n"
    );

    // base = everything up to the tail index (no deltas yet)
    let mut full = Vec::new();
    full.extend_from_slice(env.as_bytes());
    full.extend_from_slice(&body);
    let base_hash = hash_bytes(&full);
    let trailer = render_trailer(entity, 0, &base_hash, "", tail_off, tail_bytes.len() as u64);
    full.extend_from_slice(&tail_bytes);
    full.extend_from_slice(&trailer);

    std::fs::write(path, &full).map_err(|e| format!("write {path:?}: {e}"))?;
    Ok(())
}

/// Next per-writer sequence and expected prev hash for `writer`.
fn next_seq_prev(b: &EmlBox, writer: &str) -> (u64, String) {
    let mut last: Option<&TailEntry> = None;
    for e in &b.tail.entries {
        if e.writer == writer {
            last = Some(e);
        }
    }
    match last {
        Some(e) => (e.seq + 1, e.hash.clone()),
        None => (1, b.base_hash.clone()),
    }
}

/// Build a delta block for `writer` and append it. Returns (seq, block_hash).
/// Если задан ключ (EMLBOX_KEY/EMLBOX_PASS) — тело блока шифруется aes-256-gcm
/// (X-Encoding: aes), база целиком недоступна без ключа.
pub fn append_delta_w(path: &Path, writer: &str, delta: &Delta) -> Result<(u64, String), String> {
    let b = EmlBox::open(path)?;
    let entity = b.entity().ok_or("container has no X-Entity-ID")?;
    let (seq, prev) = next_seq_prev(&b, writer);
    let body = serde_json::to_vec(delta).map_err(|e| e.to_string())?;
    let (body, enc_hdr) = match crate::encoding::active_key()? {
        Some(k) => (crate::encoding::encrypt_key(&k, &body)?, "X-Encoding: aes\r\n"),
        None => (body, ""),
    };
    let mut block = format!(
        "X-EMLBox-Delta: v1\r\nX-Entity-ID: {entity}\r\nX-Writer-ID: {writer}\r\n\
         X-Delta-Seq: {seq}\r\nX-Prev-Hash: {prev}\r\n{enc_hdr}Content-Type: {DELTA_CT}\r\n\r\n"
    )
    .into_bytes();
    block.extend_from_slice(&body);
    append_block_w(path, writer, &block)
}

/// Backward-compatible local write (writer "local").
pub fn append_delta(path: &Path, delta: &Delta) -> Result<(u64, String), String> {
    append_delta_w(path, DEFAULT_WRITER, delta)
}

/// Append a block verbatim, validating its own X-Writer-ID / X-Delta-Seq /
/// X-Prev-Hash against the container state. Foreign blocks keep their bytes
/// untouched, so the writer's hash-chain stays intact across devices.
///
/// Out-of-order blocks (seq != next, or prev mismatch) are rejected — на
/// синке они остаются pending до прихода предшественника.
pub fn append_block_w(path: &Path, expected_writer: &str, block: &[u8]) -> Result<(u64, String), String> {
    let b = EmlBox::open(path)?;
    if block_header(block, "X-EMLBox-Delta").is_none() {
        return Err("block: not an X-EMLBox-Delta block".into());
    }
    let writer = block_header(block, "X-Writer-ID").unwrap_or_else(|| DEFAULT_WRITER.to_string());
    let seq: u64 = block_header(block, "X-Delta-Seq")
        .ok_or("block: no X-Delta-Seq")?
        .trim()
        .parse()
        .map_err(|_| "block: bad X-Delta-Seq")?;
    let prev = block_header(block, "X-Prev-Hash").ok_or("block: no X-Prev-Hash")?;
    let entity = b.entity().ok_or("container has no X-Entity-ID")?;

    if writer != expected_writer {
        return Err(format!("block writer {writer} != expected {expected_writer}"));
    }
    let (next_seq, expected_prev) = next_seq_prev(&b, &writer);
    if seq != next_seq {
        return Err(format!(
            "out of order: writer {writer} has seq {seq}, expected {next_seq} (missing predecessor?)"
        ));
    }
    if prev != expected_prev {
        return Err(format!("prev hash mismatch for {writer}#{seq}: got {prev}, expected {expected_prev}"));
    }

    let block_hash = hash_bytes(block);
    let mut entries = b.tail.entries.clone();
    // Deltas stay contiguous: the next block goes right after the previous one.
    let block_off = match b.tail.entries.last() {
        Some(last) => last.off + last.len,
        None => b.tail_index_off,
    };
    entries.push(TailEntry {
        seq,
        writer: writer.clone(),
        off: block_off,
        len: block.len() as u64,
        hash: block_hash.clone(),
    });
    let total = entries.len() as u64; // X-Tail-Seq = число ВСЕХ блоков (total)
    let new_tail = TailIndex { v: 1, entries };
    let new_tail_bytes = serde_json::to_vec(&new_tail).map_err(|e| e.to_string())?;
    let new_tail_off = block_off + block.len() as u64;
    let trailer = render_trailer(
        &entity,
        total,
        &b.base_hash,
        &block_hash,
        new_tail_off,
        new_tail_bytes.len() as u64,
    );

    let mut f = OpenOptions::new().write(true).open(path).map_err(|e| e.to_string())?;
    f.seek(SeekFrom::Start(block_off)).map_err(|e| e.to_string())?;
    f.write_all(block).map_err(|e| e.to_string())?;
    f.write_all(&new_tail_bytes).map_err(|e| e.to_string())?;
    f.write_all(&trailer).map_err(|e| e.to_string())?;
    f.set_len(new_tail_off + new_tail_bytes.len() as u64 + TRAILER_SIZE as u64)
        .map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    Ok((seq, block_hash))
}

/// Low-level append of a raw delta block (any EOL style — offsets are byte-exact).
/// Writer is taken from the block itself (default "local"). Returns (seq, block_hash).
pub fn append_block(path: &Path, block: &[u8]) -> Result<(u64, String), String> {
    let writer = block_header(block, "X-Writer-ID").unwrap_or_else(|| DEFAULT_WRITER.to_string());
    append_block_w(path, &writer, block)
}
