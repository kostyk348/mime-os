//! Клеточный реверс: objdump-листинг -> .eml-граф, волна типов, кластеры.

use emlbox::rev;
use std::path::PathBuf;

const LISTING: &str = r#"
0000000000001119 <net_send>:
    1119:	push   rbp
    111a:	call   1050 <helper>
    111f:	ret

000000000000111b <player_move>:
    111b:	call   1119 <net_send>
    1120:	ret

0000000000001120 <main>:
    1120:	call   111b <player_move>
    1125:	call   1119 <net_send>
    112a:	ret
"#;

fn cells(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("emlbox_rev_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn parse_listing_builds_call_graph() {
    let f = rev::parse_listing(LISTING);
    assert_eq!(f.len(), 3);
    let main = f.iter().find(|x| x.name == "main").unwrap();
    assert!(main.callees.contains(&"player_move".to_string()));
    assert!(main.callees.contains(&"net_send".to_string()));
    let pm = f.iter().find(|x| x.name == "player_move").unwrap();
    assert!(pm.callees.contains(&"net_send".to_string()));
}

#[test]
fn build_creates_eml_cells_with_references() {
    let dir = cells("build");
    let n = rev::build(&dir, LISTING).unwrap();
    assert_eq!(n, 3);
    assert!(dir.join("main.eml").exists());
    assert!(dir.join("player_move.eml").exists());
    // References: кто вызывает player_move — только main
    let refs = rev::callers(&dir, "player_move").unwrap();
    assert_eq!(refs, vec!["main".to_string()]);
}

#[test]
fn wave_propagates_types_up_the_graph() {
    let dir = cells("wave");
    rev::build(&dir, LISTING).unwrap();
    // такт 0: только net_send
    let hit = rev::type_mark(&dir, "net_send", "arg0", "Packet", 0).unwrap();
    assert_eq!(hit, vec!["net_send (arg0)".to_string()]);
    // волна 2: player_move, main (вызывающие цепочкой, прямая передача)
    let hit = rev::type_mark(&dir, "net_send", "arg0", "Packet", 2).unwrap();
    assert_eq!(hit.len(), 3);
    assert_eq!(hit[0], "net_send (arg0)");
    assert!(hit.contains(&"main (arg0)".to_string()));
    assert!(hit.contains(&"player_move (arg0)".to_string()));
    let tm = rev::type_map(&dir).unwrap();
    assert_eq!(tm.len(), 3);
    for (name, types) in &tm {
        assert!(types.contains(&"X-Type-arg0=Packet".to_string()), "{name}: {types:?}");
    }
}

#[test]
fn cluster_finds_by_name_and_body() {
    let dir = cells("cluster");
    rev::build(&dir, LISTING).unwrap();
    let hits = rev::cluster(&dir, "player").unwrap();
    // main тоже матчится: его тело содержит call <player_move>
    assert_eq!(hits, vec!["main".to_string(), "player_move".to_string()]);
    // main вызывает net_send — в теле есть упоминание
    let hits = rev::cluster(&dir, "net_send").unwrap();
    assert!(hits.contains(&"main".to_string()));
    assert!(hits.contains(&"net_send".to_string()));
}
