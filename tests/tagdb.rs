//! eml-tag: плоская теговая БД — header-only scan, X-Query, corruption-proof.

use emlbox::tagdb;
use emlbox::query;
use serde_json::json;
use std::path::PathBuf;

fn setup(tag: &str) -> PathBuf {
    let db = std::env::temp_dir().join(format!("emlbox_tagdb_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&db);
    std::fs::create_dir_all(&db).unwrap();
    db
}

fn seed(db: &PathBuf, n: usize) {
    for i in 0..n {
        let tags = vec![
            "sensor_data".to_string(),
            if i % 10 == 0 { "telemetry".to_string() } else { "status_ok".to_string() },
        ];
        tagdb::insert(db, &format!("rec_{i:04}"), &tags, "node_01", 1786528800 + i as u64, &json!({"temp": 24.5})).unwrap();
    }
}

#[test]
fn insert_and_header_scan() {
    let db = setup("scan");
    seed(&db, 100);
    let (records, corrupt) = tagdb::scan(&db).unwrap();
    assert_eq!(records.len(), 100);
    assert_eq!(corrupt, 0);
    // headers contain the fields; body must NOT be parsed into headers
    let (_, h) = &records[0];
    assert!(h.iter().any(|(k, _)| k.eq_ignore_ascii_case("X-Record-ID")));
    assert!(h.iter().any(|(k, _)| k.eq_ignore_ascii_case("X-Timestamp")));
    assert_eq!(h.iter().filter(|(k, _)| k.eq_ignore_ascii_case("X-Tag")).count(), 2);
}

#[test]
fn query_tag_device_timestamp_range() {
    let db = setup("query");
    seed(&db, 100);

    let (m, _) = tagdb::query(&db, "X-Tag == \"telemetry\"").unwrap();
    assert_eq!(m.len(), 10, "every 10th record is telemetry");

    let (m, _) = tagdb::query(&db, "X-Tag == \"telemetry\" AND X-Device-ID == \"node_01\"").unwrap();
    assert_eq!(m.len(), 10);

    let (m, _) = tagdb::query(&db, "X-Timestamp >= 1786528850 AND X-Timestamp < 1786528900").unwrap();
    assert_eq!(m.len(), 50);

    let (m, _) = tagdb::query(&db, "X-Tag == \"status_ok\" AND X-Tag == \"telemetry\"").unwrap();
    assert_eq!(m.len(), 0, "no record has both tags");
}

#[test]
fn corruption_is_isolated() {
    let db = setup("corrupt");
    seed(&db, 50);
    // trash one file: it becomes unparseable
    std::fs::write(db.join("rec_0007.eml"), b"\x00\x01\x02 no headers no blank line").unwrap();
    let (records, corrupt) = tagdb::scan(&db).unwrap();
    assert_eq!(corrupt, 1);
    assert_eq!(records.len(), 49, "49 healthy records still readable");
    // query still works, skipping the corrupted one
    let (m, corrupt) = tagdb::query(&db, "X-Tag == \"telemetry\"").unwrap();
    assert_eq!(m.len(), 5);
    assert_eq!(corrupt, 1);
}

#[test]
fn insert_is_atomic_no_tmp_left() {
    let db = setup("atomic");
    seed(&db, 10);
    let leftovers: Vec<_> = std::fs::read_dir(&db)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "no .tmp files must remain: {leftovers:?}");
}

#[test]
fn x_query_numeric_operators() {
    let h = vec![
        ("X-Timestamp".into(), "100".into()),
        ("X-Tag".into(), "a".into()),
    ];
    assert!(query::eval_headers(&h, "X-Timestamp > 99").unwrap());
    assert!(query::eval_headers(&h, "X-Timestamp <= 100").unwrap());
    assert!(!query::eval_headers(&h, "X-Timestamp < 100").unwrap());
}
