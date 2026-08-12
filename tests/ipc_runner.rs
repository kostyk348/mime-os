//! EML-IPC + EML-Runner: события шины -> исполнение logic -> дельты в контейнер.

use emlbox::{demo, ipc, kv, reader::EmlBox, runner};
use serde_json::json;
use std::path::PathBuf;

fn setup(tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("emlbox_ipc_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cont = dir.join("game.eml");
    demo::build_demo(&cont, false).unwrap();
    let bus = dir.join("bus");
    (cont, bus)
}

#[test]
fn ipc_roundtrip() {
    let (_, bus) = setup("roundtrip");
    ipc::send(&bus, "view", "game_arcade_v1", "MOVE", &json!({"dx": 5})).unwrap();
    let msgs = ipc::list(&bus).unwrap();
    assert_eq!(msgs.len(), 1);
    let m = ipc::parse(&msgs[0]).unwrap();
    assert_eq!(m.to, "game_arcade_v1");
    assert_eq!(m.event, "MOVE");
    assert_eq!(m.body, json!({"dx": 5}));
    assert!(m.from.contains("view"));
}

#[test]
fn runner_processes_move_updates_state() {
    let (cont, bus) = setup("move");
    ipc::send(&bus, "view", "game_arcade_v1", "MOVE", &json!({"dx": 5, "dy": -3})).unwrap();
    let n = runner::run(&cont, &bus, true, 10).unwrap();
    assert_eq!(n, 1, "one event processed");
    let b = EmlBox::open(&cont).unwrap();
    assert_eq!(kv::get(&b, "state", "x").unwrap(), Some(json!(47))); // 42+5
    assert_eq!(kv::get(&b, "state", "y").unwrap(), Some(json!(105))); // 108-3
    // message consumed
    assert_eq!(ipc::list(&bus).unwrap().len(), 0);
    // container still verifies clean (delta chain intact)
    assert!(emlbox::verify::verify(&cont).unwrap().is_empty());
}

#[test]
fn runner_fire_emits_outbound_message() {
    let (cont, bus) = setup("fire");
    ipc::send(&bus, "view", "game_arcade_v1", "FIRE", &json!({})).unwrap();
    let n = runner::run(&cont, &bus, true, 10).unwrap();
    assert_eq!(n, 1);
    let b = EmlBox::open(&cont).unwrap();
    assert_eq!(kv::get(&b, "state", "shots").unwrap(), Some(json!(1)));
    // hal.emit wrote an outbound SHOT addressed to <view>; it stays pending
    let msgs = ipc::list(&bus).unwrap();
    assert_eq!(msgs.len(), 1, "outbound SHOT must be in the bus");
    let m = ipc::parse(&msgs[0]).unwrap();
    assert_eq!(m.event, "SHOT");
    assert!(m.to.contains("view"));
    assert_eq!(m.body.get("shots"), Some(&json!(1)));
}

#[test]
fn unmatched_message_stays_pending() {
    let (cont, bus) = setup("unmatched");
    ipc::send(&bus, "x", "other_service", "IGNORE", &json!({})).unwrap();
    let n = runner::run(&cont, &bus, true, 10).unwrap();
    assert_eq!(n, 0, "no matching events");
    assert_eq!(ipc::list(&bus).unwrap().len(), 1, "unmatched stays for its owner");
}

#[test]
fn loop_mode_processes_messages_until_idle() {
    let (cont, bus) = setup("loop");
    ipc::send(&bus, "v", "game_arcade_v1", "MOVE", &json!({"dx": 1})).unwrap();
    ipc::send(&bus, "v", "game_arcade_v1", "MOVE", &json!({"dx": 2})).unwrap();
    let n = runner::run(&cont, &bus, false, 3).unwrap();
    assert_eq!(n, 2);
    let b = EmlBox::open(&cont).unwrap();
    assert_eq!(kv::get(&b, "state", "x").unwrap(), Some(json!(45))); // 42+1+2
    assert_eq!(ipc::list(&bus).unwrap().len(), 0);
}
