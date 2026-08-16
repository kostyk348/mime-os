//! Reader: mmap + envelope parse + head index + tail index.
//! Mount cost is O(envelope + index), never O(file body).

use crate::format::*;
use memmap2::Mmap;
use std::fs::File;
use std::path::Path;

pub struct EmlBox {
    pub mmap: Mmap,
    pub headers: Vec<(String, String)>,
    pub sections: Vec<SectionInfo>,
    pub tail: TailIndex,
    pub tail_index_off: u64,
    pub tail_index_len: u64,
    pub base_hash: String,
    pub tail_seq: u64,
    pub tail_hash: String,
}

impl EmlBox {
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("open {path:?}: {e}"))?;
        let mmap = unsafe { Mmap::map(&file) }.map_err(|e| format!("mmap: {e}"))?;

        let (headers, _) = parse_envelope(&mmap)?;

        let idx_off: u64 = header_get(&headers, "X-Index-Offset")
            .ok_or("no X-Index-Offset")?
            .trim()
            .parse()
            .map_err(|_| "bad X-Index-Offset")?;
        let idx_len: u64 = header_get(&headers, "X-Index-Length")
            .ok_or("no X-Index-Length")?
            .trim()
            .parse()
            .map_err(|_| "bad X-Index-Length")?;
        let head: HeadIndex =
            serde_json::from_slice(slice(&mmap, idx_off, idx_len)?).map_err(|e| format!("head index: {e}"))?;

        let n = mmap.len();
        if n < TRAILER_SIZE {
            return Err("file smaller than trailer".into());
        }
        let trailer = parse_trailer(&mmap[n - TRAILER_SIZE..])?;
        let get = |k: &str| -> Result<String, String> {
            trailer
                .iter()
                .find(|(kk, _)| kk == k)
                .map(|(_, v)| v.clone())
                .ok_or_else(|| format!("trailer: missing {k}"))
        };
        let tio: u64 = get("X-Tail-Index-Offset")?.trim().parse().map_err(|_| "bad tio")?;
        let til: u64 = get("X-Tail-Index-Length")?.trim().parse().map_err(|_| "bad til")?;
        let tail: TailIndex =
            serde_json::from_slice(slice(&mmap, tio, til)?).map_err(|e| format!("tail index: {e}"))?;
        let base_hash = get("X-Base-Hash")?;
        let tail_seq: u64 = get("X-Tail-Seq")?.trim().parse().unwrap_or(0);
        let tail_hash = get("X-Tail-Hash").unwrap_or_default();

        Ok(EmlBox {
            mmap,
            headers,
            sections: head.sections,
            tail,
            tail_index_off: tio,
            tail_index_len: til,
            base_hash,
            tail_seq,
            tail_hash,
        })
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        header_get(&self.headers, name)
    }

    pub fn entity(&self) -> Option<String> {
        self.header("X-Entity-ID").map(|s| s.trim().to_string())
    }

    /// Zero-copy payload slice for a base section.
    /// Декодированное содержимое секции (X-Encoding: raw|deflate|aes).
    /// None — секция не найдена ИЛИ ошибка декодирования.
    pub fn section(&self, id: &str) -> Option<Vec<u8>> {
        self.section_checked(id).ok()
    }

    /// Как section(), но ошибки декодирования не скрываются.
    pub fn section_checked(&self, id: &str) -> Result<Vec<u8>, String> {
        let s = self
            .sections
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| format!("no section '{id}'"))?;
        let raw = slice(&self.mmap, s.off, s.len)?;
        crate::encoding::decode(&s.enc, raw)
    }

    /// Zero-copy slice of a delta block.
    pub fn delta(&self, seq: u64) -> Option<&[u8]> {
        let e = self.tail.entries.iter().find(|e| e.seq == seq)?;
        slice(&self.mmap, e.off, e.len).ok()
    }

    pub fn tail_entries(&self) -> &[TailEntry] {
        &self.tail.entries
    }
}
