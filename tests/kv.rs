//! KV time travel: the append-only delta log is a history, so the state can
//! be reconstructed at any point — and the audit log reads straight from the
//! file.

use emlbox::format::Delta;
use emlbox::writer::{append_delta_w, build_file, Part};
use emlbox::{kv, reader};
use serde_json::json;
use std::path::{Path, PathBuf};

/// Append a `set` delta with an explicit timestamp.
///
/// `kv::set_w` stamps `now()` in whole seconds, so the LWW tiebreak of a test
/// that writes three deltas back to back depends on whether the run happens to
/// cross a second boundary — and it does, on a slow runner. The contract under
/// test is `(ts, writer)`, so the timestamps are given, not measured.
fn set_at(path: &Path, writer: &str, key: &str, value: serde_json::Value, ts: u64) {
    let d = Delta {
        op: "set".to_string(),
        table: "state".to_string(),
        key: key.to_string(),
        value,
        ts,
        id: None,
        after: None,
    };
    append_delta_w(path, writer, &d).unwrap();
}

fn tmp(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("emlbox_kvtime_{}_{}.eml", name, std::process::id()));
    let _ = std::fs::remove_file(&p);
    p
}

fn fresh(name: &str) -> PathBuf {
    let p = tmp(name);
    build_file(
        &p,
        "game@system.local",
        "t",
        vec![Part::raw(
            "state",
            "application/json",
            "state.json",
            br#"{}"#.to_vec(),
        )],
    )
    .unwrap();
    p
}

#[test]
fn time_travel_over_set_and_del() {
    let p = fresh("setdel");
    kv::set_w(&p, "devA", "state", "hp", json!(100)).unwrap();
    kv::set_w(&p, "devA", "state", "hp", json!(80)).unwrap();
    kv::set_w(&p, "devA", "state", "mp", json!(50)).unwrap();
    kv::del(&p, "state", "mp").unwrap();

    let b = reader::EmlBox::open(&p).unwrap();
    // now: hp=80, mp gone
    assert_eq!(kv::get(&b, "state", "hp").unwrap(), Some(json!(80)));
    assert_eq!(kv::get(&b, "state", "mp").unwrap(), None);
    // prefixes of the same log
    assert_eq!(kv::get_at(&b, "state", "hp", 0).unwrap(), None, "base only");
    assert_eq!(kv::get_at(&b, "state", "hp", 1).unwrap(), Some(json!(100)));
    assert_eq!(kv::get_at(&b, "state", "hp", 2).unwrap(), Some(json!(80)));
    assert_eq!(kv::get_at(&b, "state", "mp", 3).unwrap(), Some(json!(50)));
    assert_eq!(
        kv::get_at(&b, "state", "mp", 4).unwrap(),
        None,
        "deleted at 4"
    );
    assert!(
        kv::get_at(&b, "state", "hp", 5).is_err(),
        "past the end is an error"
    );
    drop(b);
    std::fs::remove_file(&p).ok();
}

#[test]
fn time_travel_over_rga_lists() {
    let p = fresh("rga");
    kv::add(&p, "devA", "list", "items", json!("a"), None).unwrap();
    kv::add(&p, "devA", "list", "items", json!("b"), None).unwrap();
    kv::add(&p, "devA", "list", "items", json!("c"), None).unwrap();

    let b = reader::EmlBox::open(&p).unwrap();
    let vals = |n: usize| -> Vec<String> {
        let t = kv::table_at(&b, "list", n).unwrap();
        t.get("items")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .map(|e| e["v"].as_str().unwrap().to_string())
                    .collect()
            })
            .unwrap_or_default()
    };
    assert!(vals(0).is_empty());
    assert_eq!(vals(1), vec!["a"]);
    assert_eq!(vals(2), vec!["a", "b"]);
    assert_eq!(vals(3), vec!["a", "b", "c"]);
    drop(b);
    std::fs::remove_file(&p).ok();
}

#[test]
fn history_is_the_audit_log() {
    let p = fresh("hist");
    kv::set_w(&p, "devA", "state", "hp", json!(100)).unwrap();
    kv::set_w(&p, "devB", "state", "hp", json!(80)).unwrap();
    kv::del(&p, "state", "mp").unwrap();

    let b = reader::EmlBox::open(&p).unwrap();
    let h = kv::history(&b, "state", None).unwrap();
    assert_eq!(h.len(), 3);
    assert_eq!(h[0].writer, "devA");
    assert_eq!(h[0].op, "set");
    assert_eq!(h[0].value, json!(100));
    assert_eq!(h[1].writer, "devB");
    assert_eq!(h[2].op, "del");

    // filtered by key: the mp del only
    let h = kv::history(&b, "state", Some("mp")).unwrap();
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].op, "del");
    // unknown table: empty, not an error
    assert!(kv::history(&b, "nope", None).unwrap().is_empty());
    drop(b);
    std::fs::remove_file(&p).ok();
}

#[test]
fn time_travel_across_writers_is_deterministic() {
    let p = fresh("writers");
    // same (ts) on every write: the ordering must come from the writer id alone
    set_at(&p, "devA", "x", json!(1), 1_000);
    set_at(&p, "devB", "x", json!(2), 1_000);
    set_at(&p, "devA", "x", json!(3), 1_000);

    let b = reader::EmlBox::open(&p).unwrap();
    // the replay order is fixed by (ts, writer): same prefixes on every read
    let first = kv::table_at(&b, "state", 2).unwrap();
    let second = kv::table_at(&b, "state", 2).unwrap();
    assert_eq!(first, second);
    // LWW contract: same-ts writes resolve by writer id (devB > devA),
    // so the final value is 2 no matter which writer appended last
    let final_v = kv::get(&b, "state", "x").unwrap();
    assert_eq!(final_v, Some(json!(2)));
    assert_eq!(kv::get_at(&b, "state", "x", 3).unwrap(), final_v);
    drop(b);
    std::fs::remove_file(&p).ok();
}

/// `kv::set_w` stamps the wall clock in whole seconds — the second-granular `ts`
/// is what makes the tiebreak above necessary. Pin that shape down so a later
/// change to milliseconds (or to a monotonic counter) is a deliberate one.
#[test]
fn set_w_stamps_whole_seconds() {
    let p = fresh("stamp");
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    kv::set_w(&p, "devA", "state", "x", json!(1)).unwrap();
    let after = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let b = reader::EmlBox::open(&p).unwrap();
    let h = kv::history(&b, "state", Some("x")).unwrap();
    assert_eq!(h.len(), 1);
    assert!(
        h[0].ts >= before && h[0].ts <= after,
        "ts {} outside [{before}, {after}]",
        h[0].ts
    );
    drop(b);
    std::fs::remove_file(&p).ok();
}
