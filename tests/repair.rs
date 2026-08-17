//! repair: восстановление после tear-write.

use emlbox::kv;
use emlbox::repair;
use emlbox::verify;
use emlbox::writer::{build_file, Part};
use serde_json::json;
use std::path::PathBuf;

#[test]
fn repair_recovers_after_tear_write() {
    let dir = std::env::temp_dir().join(format!("emlbox_repair_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("db.eml");
    build_file(&p, "db@system.local", "db", vec![Part::raw("state", "application/json", "state.json", br#"{"n":0}"#.to_vec())]).unwrap();
    for i in 1..=5 {
        kv::set(&p, "state", "n", json!(i)).unwrap();
    }

    // эмуляция краха: обрезаем посреди последнего блока (tail+trailer исчезли)
    let data = std::fs::read(&p).unwrap();
    let last = data.iter().rposition(|b| *b == b'X').unwrap();
    // найдём маркер последнего блока
    let marker = b"X-EMLBox-Delta: v1";
    let mut last_marker = 0;
    for i in 0..data.len().saturating_sub(marker.len()) {
        if &data[i..i + marker.len()] == marker {
            last_marker = i;
        }
    }
    let cut = last_marker + 150; // посреди последнего блока
    let crashed = dir.join("crashed.eml");
    std::fs::write(&crashed, &data[..cut]).unwrap();

    // до repair файл не читается
    assert!(emlbox::reader::EmlBox::open(&crashed).is_err());

    // repair
    let (blocks, _) = repair::repair(&crashed).unwrap();
    assert!(blocks >= 4, "восстановлено {blocks} целых блоков");

    // после: verify чистый, данные консистентны
    assert!(verify::verify(&crashed).unwrap().is_empty());
    let b = emlbox::reader::EmlBox::open(&crashed).unwrap();
    let n = kv::get(&b, "state", "n").unwrap().and_then(|v| v.as_i64()).unwrap_or(0);
    assert!(n >= 4, "n={n}");
    let _ = last;
}
