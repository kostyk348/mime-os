//! RGA-списки: конфликт-free добавление, реплики сходятся.

use emlbox::kv;
use emlbox::sync;
use emlbox::verify;
use emlbox::writer::{build_file, Part};
use serde_json::json;
use std::path::PathBuf;

fn setup(tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("emlbox_rga_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let base = dir.join("base.eml");
    build_file(&base, "g@system.local", "g", vec![Part::raw("log", "application/json", "log.json", br#"{"moves":[]}"#.to_vec())]).unwrap();
    let a = dir.join("a.eml");
    let b = dir.join("b.eml");
    std::fs::copy(&base, &a).unwrap();
    std::fs::copy(&base, &b).unwrap();
    (a, b)
}

#[test]
fn rga_lists_converge_after_merge() {
    let (a, b) = setup("merge");
    kv::add(&a, "devA", "log", "moves", json!({"x": 1}), None).unwrap();
    kv::add(&a, "devA", "log", "moves", json!({"x": 2}), None).unwrap();
    kv::add(&b, "devB", "log", "moves", json!({"x": 100}), None).unwrap();

    sync::push(&b, "devB", &a.parent().unwrap().join("bus"), "*", 0).unwrap();
    sync::pull(&a, &a.parent().unwrap().join("bus")).unwrap();
    sync::push(&a, "devA", &a.parent().unwrap().join("bus"), "*", 0).unwrap();
    sync::pull(&b, &a.parent().unwrap().join("bus")).unwrap();

    let ba = emlbox::reader::EmlBox::open(&a).unwrap();
    let bb = emlbox::reader::EmlBox::open(&b).unwrap();
    let la = kv::list(&ba, "log", "moves").unwrap();
    let lb = kv::list(&bb, "log", "moves").unwrap();
    assert_eq!(la, lb, "списки сошлись");
    assert_eq!(la.len(), 3);
    assert!(verify::verify(&a).unwrap().is_empty());
    assert!(verify::verify(&b).unwrap().is_empty());
}

#[test]
fn rga_insert_after_specific_element() {
    let (a, _) = setup("after");
    let (_, h1) = kv::add(&a, "devA", "log", "moves", json!({"n": 1}), None).unwrap();
    let _ = h1;
    // id первого элемента = "devA#1"
    kv::add(&a, "devA", "log", "moves", json!({"n": 3}), None).unwrap();
    // вставить {"n":2} ПОСЛЕ devA#1 → [1, 2, 3]
    kv::add(&a, "devA", "log", "moves", json!({"n": 2}), Some("devA#1".into())).unwrap();
    let b = emlbox::reader::EmlBox::open(&a).unwrap();
    let l = kv::list(&b, "log", "moves").unwrap();
    let vals: Vec<i64> = l.iter().map(|v| v["n"].as_i64().unwrap()).collect();
    assert_eq!(vals, vec![1, 2, 3], "{vals:?}");
}
