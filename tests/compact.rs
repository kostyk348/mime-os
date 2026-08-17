//! Compaction: дельты -> base.

use emlbox::compact;
use emlbox::kv;
use emlbox::verify;
use emlbox::writer::{build_file, Part};
use serde_json::json;
use std::path::PathBuf;

#[test]
fn compact_preserves_state_and_clears_deltas() {
    let dir = std::env::temp_dir().join(format!("emlbox_compact_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("db.eml");
    build_file(&p, "db@system.local", "db", vec![Part::raw("state", "application/json", "state.json", br#"{"n":0}"#.to_vec())]).unwrap();

    kv::set_w(&p, "devA", "state", "n", json!(1)).unwrap();
    kv::set_w(&p, "devA", "state", "n", json!(2)).unwrap();
    kv::set_w(&p, "devB", "state", "extra", json!(9)).unwrap(); // новая таблица? нет — в state
    kv::set_w(&p, "devB", "newtable", "k", json!(42)).unwrap(); // новая таблица

    let out = dir.join("slim.eml");
    let (sections, deltas) = compact::compact(&p, &out).unwrap();
    assert_eq!(deltas, 4);
    assert!(sections >= 2, "state + newtable");

    let b = emlbox::reader::EmlBox::open(&out).unwrap();
    assert_eq!(kv::get(&b, "state", "n").unwrap(), Some(json!(2)));
    assert_eq!(kv::get(&b, "state", "extra").unwrap(), Some(json!(9)));
    assert_eq!(kv::get(&b, "newtable", "k").unwrap(), Some(json!(42)));
    assert_eq!(b.tail_entries().len(), 0, "дельты слиты");
    assert!(verify::verify(&out).unwrap().is_empty());
}
