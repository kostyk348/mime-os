//! EML-FS: header-scan index, X-Query, виртуальные директории.

use emlbox::fs;
use emlbox::writer::{build_file, Part};
use std::path::PathBuf;

fn setup(tag: &str) -> PathBuf {
    let store = std::env::temp_dir().join(format!("emlbox_fs_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&store);
    std::fs::create_dir_all(&store).unwrap();
    // three containers
    build_file(
        &store.join("game.eml"),
        "game_arcade_v1@system.local",
        "Retro Arcade",
        vec![Part::raw("state", "application/json", "state.json", br#"{"x":0}"#.to_vec())],
    )
    .unwrap();
    build_file(
        &store.join("audio.eml"),
        "audio_engine@system.local",
        "Audio Engine",
        vec![Part::raw("logic", "text/x-python", "logic.py", b"".to_vec())],
    )
    .unwrap();
    build_file(
        &store.join("manual.eml"),
        "manual@system.local",
        "User Manual",
        vec![Part::raw("doc", "text/markdown", "doc.md", b"# manual".to_vec())],
    )
    .unwrap();
    store
}

#[test]
fn index_finds_containers_and_skips_non_containers() {
    let store = setup("idx");
    // a non-container .eml in the store
    std::fs::write(store.join("junk.eml"), "this is not an emlbox container").unwrap();

    let (recs, skipped) = fs::index(&store).unwrap();
    assert_eq!(recs.len(), 3);
    assert_eq!(skipped.len(), 1, "junk.eml must be skipped");
    let game = recs.iter().find(|r| r.entity == "game_arcade_v1@system.local").unwrap();
    assert_eq!(game.eml_type, "Application/Unified");
    assert_eq!(game.subject, "Retro Arcade");
}

#[test]
fn tags_via_kv_delta_are_indexed() {
    let store = setup("tags");
    // tag game + audio dynamically (append deltas to their tags tables)
    let (recs, _) = fs::index(&store).unwrap();
    let game = recs.iter().find(|r| r.entity == "game_arcade_v1@system.local").unwrap();
    let audio = recs.iter().find(|r| r.entity == "audio_engine@system.local").unwrap();
    fs::tag(&store, &game.entity, "game").unwrap();
    fs::tag(&store, &game.entity, "arcade").unwrap();
    fs::tag(&store, &audio.entity, "game").unwrap();
    fs::tag(&store, &audio.entity, "audio").unwrap();

    let (recs, _) = fs::index(&store).unwrap();
    let game = recs.iter().find(|r| r.entity == "game_arcade_v1@system.local").unwrap();
    let audio = recs.iter().find(|r| r.entity == "audio_engine@system.local").unwrap();
    assert_eq!(game.tags, vec!["arcade", "game"]);
    assert_eq!(audio.tags, vec!["audio", "game"]);
}

#[test]
fn x_query_and_and_not() {
    let store = setup("query");
    let (recs, _) = fs::index(&store).unwrap();

    let m = fs::eval(&recs, "X-Tag == \"game\"").unwrap();
    assert_eq!(m.len(), 0, "no tags yet");

    // tag game only
    let game = recs.iter().find(|r| r.entity == "game_arcade_v1@system.local").unwrap();
    fs::tag(&store, &game.entity, "game").unwrap();
    let (recs, _) = fs::index(&store).unwrap();

    let m = fs::eval(&recs, "X-Tag == \"game\"").unwrap();
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].entity, "game_arcade_v1@system.local");

    let m = fs::eval(&recs, "X-Tag == \"game\" AND X-EML-Type == \"Application/Unified\"").unwrap();
    assert_eq!(m.len(), 1);

    let m = fs::eval(&recs, "X-Tag != \"game\"").unwrap();
    assert_eq!(m.len(), 2);

    let m = fs::eval(&recs, "Subject == \"manual\"").unwrap();
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].entity, "manual@system.local");

    assert!(fs::eval(&recs, "bogus").is_err(), "bad clause must error");
}

#[test]
fn virtual_directory_query_and_contains() {
    let store = setup("dirs");
    // tag game + audio as "game"
    let (recs, _) = fs::index(&store).unwrap();
    for r in &recs {
        if r.entity.contains("game") || r.entity.contains("audio") {
            fs::tag(&store, &r.entity, "game").unwrap();
        }
    }

    // dynamic dir: X-Query X-Tag == "game"
    fs::mkdir(&store, "games", Some("X-Tag == \"game\""), &[]).unwrap();
    // explicit dir: X-Contains-ID
    fs::mkdir(&store, "docs", None, &["manual@system.local".to_string()]).unwrap();

    let (recs, _) = fs::index(&store).unwrap();
    assert_eq!(recs.len(), 5, "3 containers + 2 directories");

    let games = recs.iter().find(|r| r.entity == "games@system.local").unwrap();
    assert_eq!(games.eml_type, "System/Directory");
    let members: Vec<String> = fs::resolve(&recs, games).iter().map(|r| r.entity.clone()).collect();
    assert_eq!(members.len(), 2, "game + audio match the query");
    assert!(members.iter().any(|e| e == "game_arcade_v1@system.local"));
    assert!(members.iter().any(|e| e == "audio_engine@system.local"));

    let docs = recs.iter().find(|r| r.entity == "docs@system.local").unwrap();
    let members: Vec<String> = fs::resolve(&recs, docs).iter().map(|r| r.entity.clone()).collect();
    assert_eq!(members, vec!["manual@system.local"]);

    // dir with both: union, no dupes
    fs::mkdir(
        &store,
        "mixed",
        Some("X-Tag == \"game\""),
        &["manual@system.local".to_string()],
    )
    .unwrap();
    let (recs, _) = fs::index(&store).unwrap();
    let mixed = recs.iter().find(|r| r.entity == "mixed@system.local").unwrap();
    let members = fs::resolve(&recs, mixed);
    assert_eq!(members.len(), 3, "union of query + explicit, no dupes");
}

#[test]
fn directory_does_not_match_tag_query() {
    let store = setup("norec");
    let (recs, _) = fs::index(&store).unwrap();
    for r in &recs {
        fs::tag(&store, &r.entity, "game").unwrap();
    }
    fs::mkdir(&store, "games", Some("X-Tag == \"game\""), &[]).unwrap();
    let (recs, _) = fs::index(&store).unwrap();
    // directories have no tags -> only the 3 containers match
    let m = fs::eval(&recs, "X-Tag == \"game\"").unwrap();
    assert_eq!(m.len(), 3);
    assert!(m.iter().all(|r| r.eml_type != "System/Directory"));
}
