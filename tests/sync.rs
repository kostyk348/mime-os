//! Сетевая фаза: multi-writer delta-sync.

use emlbox::kv;
use emlbox::sync;
use emlbox::verify;
use emlbox::writer::{append_delta_w, build_file, Part};
use serde_json::json;
use std::path::PathBuf;

fn setup(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("emlbox_sync_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let base = dir.join("base.eml");
    build_file(
        &base,
        "game@system.local",
        "game",
        vec![Part::raw("state", "application/json", "state.json", br#"{"x":0,"y":0}"#.to_vec())],
    )
    .unwrap();
    let a = dir.join("devA.eml");
    let b = dir.join("devB.eml");
    std::fs::copy(&base, &a).unwrap();
    std::fs::copy(&base, &b).unwrap();
    (a, b, dir.join("bus"))
}

#[test]
fn two_devices_merge_chains() {
    let (a, b, bus) = setup("merge");
    kv::set_w(&a, "devA", "state", "x", json!(1)).unwrap();
    kv::set_w(&a, "devA", "state", "x", json!(2)).unwrap();
    kv::set_w(&b, "devB", "state", "y", json!(9)).unwrap();

    // B -> bus -> A
    sync::push(&b, "devB", &bus, "game@system.local", 0).unwrap();
    let (applied, pending) = sync::pull(&a, &bus).unwrap();
    assert_eq!(applied, 1);
    assert_eq!(pending, 0);

    assert!(verify::verify(&a).unwrap().is_empty(), "A: {:?}", verify::verify(&a).unwrap());
    assert!(verify::verify(&b).unwrap().is_empty());

    let bbox = emlbox::reader::EmlBox::open(&a).unwrap();
    assert_eq!(kv::get(&bbox, "state", "x").unwrap(), Some(json!(2)));
    assert_eq!(kv::get(&bbox, "state", "y").unwrap(), Some(json!(9)));

    // обе цепочки на месте
    let h = sync::heads(&a).unwrap();
    assert_eq!(h.len(), 2);
    assert_eq!(h[0].0, "devA");
    assert_eq!(h[1].0, "devB");

    // обратный синк: A -> bus -> B
    sync::push(&a, "devA", &bus, "game@system.local", 0).unwrap();
    let (applied, _) = sync::pull(&b, &bus).unwrap();
    assert_eq!(applied, 2);
    assert!(verify::verify(&b).unwrap().is_empty());
    let bbox = emlbox::reader::EmlBox::open(&b).unwrap();
    assert_eq!(kv::get(&bbox, "state", "x").unwrap(), Some(json!(2)));
    assert_eq!(kv::get(&bbox, "state", "y").unwrap(), Some(json!(9)));
}

#[test]
fn conflict_lww_by_timestamp() {
    let (a, b, bus) = setup("lww");
    let d1 = emlbox::format::Delta { op: "set".into(), table: "state".into(), key: "k".into(), value: json!("fromA"), ts: 100 };
    let d2 = emlbox::format::Delta { op: "set".into(), table: "state".into(), key: "k".into(), value: json!("fromB"), ts: 200 };
    append_delta_w(&a, "devA", &d1).unwrap();
    append_delta_w(&b, "devB", &d2).unwrap();
    sync::push(&b, "devB", &bus, "game@system.local", 0).unwrap();
    sync::pull(&a, &bus).unwrap();
    // позже по времени выигрывает детерминированно
    let bbox = emlbox::reader::EmlBox::open(&a).unwrap();
    assert_eq!(kv::get(&bbox, "state", "k").unwrap(), Some(json!("fromB")));

    // и наоборот: если бы devA был позже — выиграл бы он
    let (a2, b2, bus2) = setup("lww2");
    let d1 = emlbox::format::Delta { op: "set".into(), table: "state".into(), key: "k".into(), value: json!("fromA"), ts: 300 };
    append_delta_w(&a2, "devA", &d1).unwrap();
    append_delta_w(&b2, "devB", &d2).unwrap();
    sync::push(&b2, "devB", &bus2, "game@system.local", 0).unwrap();
    sync::pull(&a2, &bus2).unwrap();
    let bbox = emlbox::reader::EmlBox::open(&a2).unwrap();
    assert_eq!(kv::get(&bbox, "state", "k").unwrap(), Some(json!("fromA")));
}

#[test]
fn out_of_order_block_stays_pending() {
    let (a, b, bus) = setup("ooo");
    let d1 = emlbox::format::Delta { op: "set".into(), table: "state".into(), key: "k".into(), value: json!(1), ts: 1 };
    let d2 = emlbox::format::Delta { op: "set".into(), table: "state".into(), key: "k".into(), value: json!(2), ts: 2 };
    append_delta_w(&b, "devB", &d1).unwrap();
    append_delta_w(&b, "devB", &d2).unwrap();

    // в шину кладём только seq=2 (без seq=1)
    let blocks = sync::export(&b, "devB", 1).unwrap(); // seq 2 only
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].0, 2);
    std::fs::create_dir_all(&bus).unwrap();
    let be = emlbox::reader::EmlBox::open(&b).unwrap();
    let entity = be.entity().unwrap();
    let mut msg = format!(
        "From: <{entity}>\r\nTo: <{entity}>\r\nX-EMLBox-Sync: v1\r\nX-Writer-ID: devB\r\nContent-Type: application/x-emlbox-delta\r\n\r\n"
    )
    .into_bytes();
    msg.extend_from_slice(&blocks[0].1);
    std::fs::write(bus.join("ooo.msg.eml"), &msg).unwrap();

    let (applied, pending) = sync::pull(&a, &bus).unwrap();
    assert_eq!(applied, 0, "seq=2 без seq=1 применить нельзя");
    assert_eq!(pending, 1, "блок ждёт seq=1");

    // теперь приходит seq=1 — применяются оба новых; дубликат seq=2 дедуплицируется
    sync::push(&b, "devB", &bus, "game@system.local", 0).unwrap();
    let (applied, pending) = sync::pull(&a, &bus).unwrap();
    assert_eq!(applied, 3, "seq1 + seq2 + повторный seq2 (dedup)");
    assert_eq!(pending, 0);
    let bbox = emlbox::reader::EmlBox::open(&a).unwrap();
    assert_eq!(kv::get(&bbox, "state", "k").unwrap(), Some(json!(2)));
    assert!(verify::verify(&a).unwrap().is_empty());
}

#[test]
fn export_since_seq() {
    let (a, _, _) = setup("export");
    kv::set_w(&a, "devA", "state", "a", json!(1)).unwrap();
    kv::set_w(&a, "devA", "state", "b", json!(2)).unwrap();
    let all = sync::export(&a, "devA", 0).unwrap();
    assert_eq!(all.len(), 2);
    let tail = sync::export(&a, "devA", 1).unwrap();
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].0, 2);
    let h = sync::heads(&a).unwrap();
    assert_eq!(h[0].0, "devA");
    assert_eq!(h[0].1, 2);
    assert_eq!(h[0].2, emlbox::format::hash_bytes(&all[1].1));
    assert_eq!(h.len(), 1);
}

#[test]
fn tcp_roundtrip_converges() {
    use std::net::TcpListener;
    let (a, b, _) = setup("tcp");
    kv::set_w(&a, "devA", "state", "x", json!(10)).unwrap();
    kv::set_w(&a, "devA", "state", "x", json!(11)).unwrap();
    kv::set_w(&b, "devB", "state", "y", json!(99)).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let b2 = b.clone();
    let srv = std::thread::spawn(move || {
        // one peer then stop
        let (stream, _) = listener.accept().unwrap();
        emlbox::sync::tcp_serve_once(&b2, stream).unwrap();
    });
    let (recv, sent) = sync::tcp_connect(&a, &addr.to_string()).unwrap();
    srv.join().unwrap();
    assert_eq!(recv, 1, "A получил devB#1");
    assert_eq!(sent, 2, "A отдал devA#1,#2");

    assert!(verify::verify(&a).unwrap().is_empty());
    assert!(verify::verify(&b).unwrap().is_empty());
    // обе стороны сошлись
    let ha = sync::heads(&a).unwrap();
    let hb = sync::heads(&b).unwrap();
    assert_eq!(ha.len(), 2);
    assert_eq!(hb.len(), 2);
    let ba = emlbox::reader::EmlBox::open(&a).unwrap();
    let bb = emlbox::reader::EmlBox::open(&b).unwrap();
    assert_eq!(kv::get(&ba, "state", "x").unwrap(), Some(json!(11)));
    assert_eq!(kv::get(&ba, "state", "y").unwrap(), Some(json!(99)));
    assert_eq!(kv::get(&bb, "state", "x").unwrap(), Some(json!(11)));
    assert_eq!(kv::get(&bb, "state", "y").unwrap(), Some(json!(99)));
}
