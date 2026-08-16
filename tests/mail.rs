//! SMTP-мост: контейнеры путешествуют как настоящие письма.

use emlbox::kv;
use emlbox::mail;
use emlbox::verify;
use emlbox::writer::{build_file, Part};
use serde_json::json;
use std::path::PathBuf;

fn setup(tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("emlbox_mail_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let base = dir.join("base.eml");
    build_file(&base, "game@system.local", "game", vec![Part::raw("state", "application/json", "state.json", br#"{"x":0}"#.to_vec())]).unwrap();
    let a = dir.join("devA.eml");
    let b = dir.join("devB.eml");
    std::fs::copy(&base, &a).unwrap();
    std::fs::copy(&base, &b).unwrap();
    (a, b)
}

#[test]
fn letter_carries_deltas_between_devices() {
    let (a, b) = setup("letter");
    kv::set_w(&a, "devA", "state", "x", json!(42)).unwrap();
    kv::set_w(&a, "devA", "state", "lvl", json!(7)).unwrap();

    // A пакует дельты в письмо
    let letter = mail::pack(&a, "devA", "devB@example.com", 0).unwrap();
    assert!(String::from_utf8_lossy(&letter).contains("X-EMLBox-Sync: v1"));
    assert!(String::from_utf8_lossy(&letter).contains("multipart/mixed"));

    // B применяет
    let (applied, pending) = mail::apply(&b, &letter).unwrap();
    assert_eq!(applied, 2);
    assert_eq!(pending, 0);
    let bb = emlbox::reader::EmlBox::open(&b).unwrap();
    assert_eq!(kv::get(&bb, "state", "x").unwrap(), Some(json!(42)));
    assert_eq!(kv::get(&bb, "state", "lvl").unwrap(), Some(json!(7)));
    assert!(verify::verify(&b).unwrap().is_empty());
}

#[test]
fn maildir_receive_applies_and_moves() {
    let (a, _) = setup("maildir");
    let dir = a.parent().unwrap().join("Maildir");
    let ndir = dir.join("new");
    std::fs::create_dir_all(&ndir).unwrap();

    kv::set_w(&a, "devA", "state", "x", json!(1)).unwrap();
    kv::set_w(&a, "devA", "state", "x", json!(2)).unwrap();
    // "прислали" письмо: дельта от A же (репликация между своими копиями)
    let letter = mail::pack(&a, "devA", "me@example.com", 1).unwrap();
    let lp = ndir.join("incoming.eml");
    std::fs::write(&lp, &letter).unwrap();

    let (applied, pending) = mail::receive(&a, &dir).unwrap();
    assert_eq!(applied, 1);
    assert_eq!(pending, 0);
    // письмо уехало в processed
    let proc = dir.join("processed");
    assert!(std::fs::read_dir(&proc).unwrap().next().is_some());
    assert!(!lp.exists(), "письмо перемещено");
}
