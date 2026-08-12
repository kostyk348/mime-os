//! verify: recompute index bounds, base hash, and the delta hash chain.

use crate::format::{block_header, hash_bytes, hash_slice, slice};
use crate::reader::EmlBox;
use std::path::Path;

pub fn verify(path: &Path) -> Result<Vec<String>, String> {
    let mut issues: Vec<String> = Vec::new();
    let b = EmlBox::open(path)?;

    // 1. base sections: in-bounds, non-overlapping, ordered
    let mut prev_end = 0u64;
    for s in &b.sections {
        if s.off < prev_end {
            issues.push(format!("section '{}' overlaps previous", s.id));
        }
        if s.off + s.len > b.mmap.len() as u64 {
            issues.push(format!("section '{}' out of bounds", s.id));
        }
        if s.off < 1 {
            issues.push(format!("section '{}' offset too small", s.id));
        }
        prev_end = s.off + s.len;
    }

    // 2. base hash: base region = [0 .. first delta), or [0 .. tail index) if no deltas
    let base_end = match b.tail_entries().first() {
        Some(e) => e.off,
        None => b.tail_index_off,
    };
    let got = hash_slice(&b.mmap, 0, base_end)?;
    if got != b.base_hash {
        issues.push(format!("base hash mismatch: got {got}, expected {}", b.base_hash));
    }

    // 3. delta chain: each block hashes to its entry and links from base
    let mut expect = b.base_hash.clone();
    for e in b.tail_entries() {
        let block = slice(&b.mmap, e.off, e.len)?;
        let bh = hash_bytes(block);
        if bh != e.hash {
            issues.push(format!("delta {} hash mismatch", e.seq));
        }
        match block_header(block, "X-Prev-Hash") {
            Some(prev) if prev == expect => {}
            Some(prev) => issues.push(format!("delta {} chain break: prev={prev}, expected {expect}", e.seq)),
            None => issues.push(format!("delta {} missing X-Prev-Hash", e.seq)),
        }
        expect = bh;
    }

    // 4. trailer consistency
    if b.tail_seq as usize != b.tail_entries().len() {
        issues.push(format!("trailer seq {} != tail entries {}", b.tail_seq, b.tail_entries().len()));
    }
    if let Some(last) = b.tail_entries().last() {
        if !b.tail_hash.is_empty() && b.tail_hash != last.hash {
            issues.push("trailer tail hash != last delta hash".into());
        }
    }

    Ok(issues)
}
