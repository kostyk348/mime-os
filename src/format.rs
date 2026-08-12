//! EMLBox v0.1 wire format: constants, types, and low-level parsing helpers.
//!
//! Layout of an .eml container:
//!   [0 .. H)             envelope headers (RFC 822), end at first blank line
//!   [H .. I)             base multipart: data sections + head index section (last)
//!   [I .. TI)            appended delta blocks (MIME messages), absent at creation
//!   [TI .. T)            tail index JSON  (rewritten on every append)
//!   [T .. EOF)           512-byte trailer (fixed size, always at EOF)
//!
//! Invariants:
//!   * base section offsets are absolute byte offsets, stable forever (append never
//!     rewrites the base prefix);
//!   * reading is index-driven: envelope -> head index -> tail index, never a full
//!     body scan; payload slices are zero-copy mmap windows;
//!   * delta blocks form a hash chain from X-Base-Hash (see writer::append_block).

pub const VERSION: &str = "0.1.0";
pub const TRAILER_SIZE: usize = 512;
pub const TRAILER_MARKER: &str = "X-EMLBox-Trailer: v1";
pub const INDEX_CT: &str = "application/x-emlbox-index+json";
pub const TAIL_CT: &str = "application/x-emlbox-tail+json";
pub const DELTA_CT: &str = "application/x-emlbox-delta+json";
pub const ENC_RAW: &str = "raw";

use memmap2::Mmap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------- types

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SectionInfo {
    pub id: String,
    pub ct: String,
    #[serde(default)]
    pub name: String,
    /// absolute byte offset of the payload inside the file
    pub off: u64,
    pub len: u64,
    #[serde(default = "default_enc")]
    pub enc: String,
}

fn default_enc() -> String {
    ENC_RAW.to_string()
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct HeadIndex {
    #[serde(default)]
    pub v: u32,
    pub sections: Vec<SectionInfo>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TailEntry {
    pub seq: u64,
    pub off: u64,
    pub len: u64,
    pub hash: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct TailIndex {
    #[serde(default)]
    pub v: u32,
    #[serde(default)]
    pub entries: Vec<TailEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Delta {
    pub op: String, // "set" | "del"
    pub table: String,
    pub key: String,
    #[serde(default)]
    pub value: serde_json::Value,
    #[serde(default)]
    pub ts: u64,
}

// ---------------------------------------------------------------- hashing

pub fn hash_bytes(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

pub fn hash_slice(m: &Mmap, off: u64, len: u64) -> Result<String, String> {
    Ok(hash_bytes(slice(m, off, len)?))
}

// ---------------------------------------------------------------- low-level slicing

pub fn slice(m: &Mmap, off: u64, len: u64) -> Result<&[u8], String> {
    let o = off as usize;
    let l = len as usize;
    if o + l > m.len() {
        return Err(format!("slice out of bounds: off={off} len={len} file={}", m.len()));
    }
    Ok(&m[o..o + l])
}

/// Find the first blank line (CRLFCRLF or LFLF). Returns (start, separator_len).
pub fn find_blank(data: &[u8]) -> Option<(usize, usize)> {
    if data.len() >= 4 {
        for i in 0..data.len() - 3 {
            if &data[i..i + 4] == b"\r\n\r\n" {
                return Some((i, 4));
            }
        }
    }
    if data.len() >= 2 {
        for i in 0..data.len() - 1 {
            if &data[i..i + 2] == b"\n\n" {
                return Some((i, 2));
            }
        }
    }
    None
}

// ---------------------------------------------------------------- header parsing

/// Parse RFC 822 header block (bytes up to the first blank line), with
/// continuation-line folding. EOL-lenient (accepts CRLF and LF).
pub fn parse_headers(head: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(head);
    let mut out: Vec<(String, String)> = Vec::new();
    let mut last: Option<usize> = None;
    for line in text.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(i) = last {
                out[i].1.push(' ');
                out[i].1.push_str(line.trim());
            }
            continue;
        }
        if let Some(idx) = line.find(':') {
            let k = line[..idx].trim().to_string();
            let v = line[idx + 1..].trim().to_string();
            out.push((k, v));
            last = Some(out.len() - 1);
        }
    }
    out
}

pub fn header_get<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

pub fn parse_envelope(m: &Mmap) -> Result<(Vec<(String, String)>, usize), String> {
    let limit = m.len().min(65536);
    let data = &m[..limit];
    let (end, sep) = find_blank(data).ok_or("envelope: no blank line after headers")?;
    let headers = parse_headers(&data[..end]);
    if headers.is_empty() {
        return Err("envelope: no headers parsed".into());
    }
    Ok((headers, end + sep))
}

/// Parse the fixed-size trailer (last TRAILER_SIZE bytes). Returns key/value map.
pub fn parse_trailer(t: &[u8]) -> Result<Vec<(String, String)>, String> {
    let text = String::from_utf8_lossy(t);
    let start = text.find(TRAILER_MARKER).ok_or("trailer marker not found")?;
    let mut out = Vec::new();
    for line in text[start..].split('\n') {
        let line = line.trim_end_matches('\r');
        if line.starts_with("X-") {
            if let Some(idx) = line.find(": ") {
                out.push((line[..idx].trim().to_string(), line[idx + 2..].trim().to_string()));
                continue;
            }
        }
        // padding reached
        if !out.is_empty() {
            break;
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- delta blocks

/// Extract a single header value from a delta block's own headers.
pub fn block_header(block: &[u8], name: &str) -> Option<String> {
    let (end, _) = find_blank(block)?;
    let headers = parse_headers(&block[..end]);
    header_get(&headers, name).map(|s| s.trim().to_string())
}

/// Parse a delta block body into a Delta. Returns Ok(None) for non-delta content.
pub fn parse_delta_block(block: &[u8]) -> Result<Option<Delta>, String> {
    if block_header(block, "X-EMLBox-Delta").is_none() {
        return Ok(None);
    }
    let (end, sep) = find_blank(block).ok_or("delta: no blank line")?;
    let body = &block[end + sep..];
    let delta: Delta = serde_json::from_slice(body).map_err(|e| format!("delta json: {e}"))?;
    Ok(Some(delta))
}

// ---------------------------------------------------------------- trailer rendering

pub fn render_trailer(
    entity: &str,
    tail_seq: u64,
    base_hash: &str,
    tail_hash: &str,
    tail_index_off: u64,
    tail_index_len: u64,
) -> Vec<u8> {
    let mut s = String::new();
    s.push_str(TRAILER_MARKER);
    s.push_str("\r\n");
    s.push_str(&format!("X-Entity-ID: {entity}\r\n"));
    s.push_str(&format!("X-Tail-Seq: {tail_seq}\r\n"));
    s.push_str(&format!("X-Tail-Hash: {tail_hash}\r\n"));
    s.push_str(&format!("X-Base-Hash: {base_hash}\r\n"));
    s.push_str(&format!("X-Tail-Index-Offset: {tail_index_off}\r\n"));
    s.push_str(&format!("X-Tail-Index-Length: {tail_index_len}\r\n"));
    let mut out = vec![b' '; TRAILER_SIZE];
    out[..s.len()].copy_from_slice(s.as_bytes());
    out
}
