//! Invariant tests: stable base offsets, delta order, hash chain, LF tolerance.

use emlbox::format::{block_header, find_blank, hash_bytes, parse_trailer, TRAILER_MARKER, TRAILER_SIZE};
use emlbox::kv;
use emlbox::reader::EmlBox;
use emlbox::verify;
use emlbox::writer::{append_block, build_file, Part};
use serde_json::json;

fn tmp(name: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("emlbox_test_{}_{name}", std::process::id()));
    let _ = std::fs::remove_file(&p);
    p
}

fn base_file(path: &std::path::Path) {
    build_file(
        path,
        "test_entity",
        "test subject",
        vec![
            Part::raw("a", "application/json", "a.json", br#"{"k":1}"#.to_vec()),
            Part::raw("b", "text/plain", "b.txt", b"hello world".to_vec()),
        ],
    )
    .unwrap();
}

#[test]
fn stable_base_offsets_after_appends() {
    let p = tmp("stable");
    base_file(&p);

    let b0 = EmlBox::open(&p).unwrap();
    let before: Vec<(String, u64, u64)> = b0.sections.iter().map(|s| (s.id.clone(), s.off, s.len)).collect();

    // 5 deltas (simulate one delta as KV set, rest raw)
    kv::set(&p, "a", "x", json!(10)).unwrap();
    kv::set(&p, "a", "y", json!(20)).unwrap();
    kv::set(&p, "a", "z", json!(30)).unwrap();

    let b1 = EmlBox::open(&p).unwrap();
    let after: Vec<(String, u64, u64)> = b1.sections.iter().map(|s| (s.id.clone(), s.off, s.len)).collect();
    assert_eq!(before, after, "base section offsets MUST NOT change after appends");
    assert_eq!(b1.tail.entries.len(), 3);

    // data still readable
    assert_eq!(b1.section("b").unwrap(), b"hello world");
    assert_eq!(kv::get(&b1, "a", "x").unwrap(), Some(json!(10)));
    assert_eq!(kv::get(&b1, "a", "z").unwrap(), Some(json!(30)));
}

#[test]
fn delta_order_and_delete() {
    let p = tmp("order");
    base_file(&p);
    kv::set(&p, "a", "n", json!(1)).unwrap();
    kv::set(&p, "a", "n", json!(2)).unwrap();
    kv::set(&p, "a", "n", json!(3)).unwrap();
    kv::del(&p, "a", "n").unwrap();

    let b = EmlBox::open(&p).unwrap();
    assert_eq!(kv::get(&b, "a", "n").unwrap(), None, "del must remove key");
    assert_eq!(kv::get(&b, "a", "k").unwrap(), Some(json!(1)), "base key intact");
}

#[test]
fn hash_chain_verifies_clean() {
    let p = tmp("chain");
    base_file(&p);
    kv::set(&p, "a", "x", json!(5)).unwrap();
    kv::set(&p, "a", "x", json!(7)).unwrap();
    let issues = verify::verify(&p).unwrap();
    assert!(issues.is_empty(), "clean container must verify: {issues:?}");
}

#[test]
fn tamper_detected_by_verify() {
    let p = tmp("tamper");
    base_file(&p);
    kv::set(&p, "a", "x", json!(5)).unwrap();

    // flip one byte inside base section "b" payload
    let data = std::fs::read(&p).unwrap();
    let b = EmlBox::open(&p).unwrap();
    let sec = b.sections.iter().find(|s| s.id == "b").unwrap();
    let mut data = data;
    data[(sec.off + 1) as usize] ^= 0xff;
    std::fs::write(&p, &data).unwrap();

    let issues = verify::verify(&p).unwrap();
    assert!(!issues.is_empty(), "tampered base must be detected");
    assert!(issues.iter().any(|i| i.contains("base hash")));
}

#[test]
fn lf_delta_block_is_tolerated() {
    let p = tmp("lf");
    base_file(&p);

    // hand-write an LF-only delta block (CRLF absent)
    let block = format!(
        "X-EMLBox-Delta: v1\nX-Entity-ID: test_entity\nX-Delta-Seq: 1\nX-Prev-Hash: {}\nContent-Type: application/x-emlbox-delta+json\n\n{}\n",
        EmlBox::open(&p).unwrap().base_hash,
        serde_json::to_string(&serde_json::json!({"op":"set","table":"a","key":"lf","value":42})).unwrap()
    )
    .into_bytes();
    append_block(&p, &block).unwrap();

    let b = EmlBox::open(&p).unwrap();
    assert_eq!(kv::get(&b, "a", "lf").unwrap(), Some(json!(42)), "LF delta must parse");
    assert!(verify::verify(&p).unwrap().is_empty());
    assert_eq!(block_header(&block, "X-Delta-Seq").unwrap(), "1");
}

#[test]
fn trailer_roundtrip() {
    let t = emlbox::format::render_trailer("e", 7, "abc", "def", 1234, 56);
    assert_eq!(t.len(), TRAILER_SIZE);
    let kv = parse_trailer(&t).unwrap();
    let map: std::collections::HashMap<String, String> = kv.into_iter().collect();
    assert_eq!(map.get("X-Entity-ID").unwrap(), "e");
    assert_eq!(map.get("X-Tail-Seq").unwrap(), "7");
    assert_eq!(map.get("X-Tail-Index-Offset").unwrap(), "1234");
}

#[test]
fn find_blank_crlf_and_lf() {
    assert_eq!(find_blank(b"a: b\r\n\r\nbody").unwrap(), (4, 4));
    assert_eq!(find_blank(b"a: b\n\nbody").unwrap(), (4, 2));
    assert_eq!(find_blank(b"no blank here"), None);
}

#[test]
fn hash_is_sha256_hex() {
    let h = hash_bytes(b"");
    assert_eq!(h.len(), 64);
    assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn mount_after_manual_eol_edit_offsets_still_authoritative() {
    // Offsets are byte offsets; rewriting EOLs breaks them by design.
    // This test documents the property: any textual edit invalidates the index.
    let p = tmp("eol");
    base_file(&p);
    let data = std::fs::read(&p).unwrap();
    let lf = String::from_utf8_lossy(&data).replace("\r\n", "\n").into_bytes();
    std::fs::write(&p, &lf).unwrap();
    // Either verification fails cleanly (base hash mismatch) or the structure
    // breaks so hard the reader errors — both prove the edit was detected.
    match verify::verify(&p) {
        Ok(issues) => assert!(!issues.is_empty(), "edited file must fail verification"),
        Err(_) => {}
    }
}
