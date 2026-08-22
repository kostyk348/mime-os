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

#[test]
fn branch_isolates_hypothesis_and_rolls_back() {
    let dir = cells("branch");
    rev::build(&dir, LISTING).unwrap();
    // ветка от player_move, depth 1: player_move@b + main@b (main вызывает player_move)
    let n = rev::branch(&dir, "player_move", "b", 1).unwrap();
    assert!(n >= 2);
    assert!(dir.join("player_move@b.eml").exists());
    assert!(dir.join("main@b.eml").exists());

    // волна в ветке: net_send@b? нет — net_send не в подграфе (вызывается, не вызывает).
    // player_move@b вызывается main@b (прямая передача) — волна от player_move@b arg0 идёт на main@b
    let hit = rev::type_mark(&dir, "player_move@b", "arg0", "Packet", 2).unwrap();
    assert!(hit.iter().any(|h| h.contains("main@b")), "{hit:?}");

    // изоляция: исходные клетки не помечены
    let tm = rev::type_map(&dir).unwrap();
    for (name, _) in &tm {
        assert!(name.contains('@'), "исходная клетка тронута: {name}");
    }

    // откат
    let removed = rev::branch_rm(&dir, "b").unwrap();
    assert_eq!(removed, n);
    assert!(!dir.join("player_move@b.eml").exists());
}

#[test]
fn recon_classifies_regions() {
    let dir = std::env::temp_dir().join(format!("emlbox_recon_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let p = dir.join("blob.bin");
    // 8192 байт plain 'A' + 8192 случайных (высокая энтропия)
    let mut data = vec![b'A'; 8192];
    for _ in 0..8192 {
        data.push((std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() % 256) as u8);
    }
    std::fs::write(&p, &data).unwrap();
    let (strings, regions) = rev::recon(&p).unwrap();
    // строки: куски 'A' длиной >= 6
    assert!(strings.len() >= 1);
    // регионы: первые ~plain, потом высокоэнтропийные
    assert!(regions.len() >= 2);
    let classes: Vec<&str> = regions.iter().map(|(_, _, _, c)| *c).collect();
    assert!(classes.iter().any(|c| *c == "plain" || *c == "code"), "{classes:?}");
    assert!(classes.last().map(|c| *c == "compressed" || *c == "code").unwrap_or(false), "{classes:?}");
}

#[test]
fn vftables_found_in_cpp_binary() {
    // требует g++ (пропускается без него)
    let gpp = std::process::Command::new("g++").arg("--version").output();
    if gpp.map(|o| !o.status.success()).unwrap_or(true) {
        return;
    }
    let dir = std::env::temp_dir().join(format!("emlbox_vft_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("s.cpp");
    let bin = dir.join("s");
    std::fs::write(&src, r#"
struct A { virtual ~A(){} virtual int f() const { return 1; } virtual int g() const { return 2; } };
struct B : A { int f() const override { return 10; } };
int main(){ A a; B b; return a.f() + b.g(); }
"#).unwrap();
    if !std::process::Command::new("g++").args(["-O0", "-g"]).arg(&src).arg("-o").arg(&bin).status().map(|s| s.success()).unwrap_or(false) {
        return;
    }
    let cands = rev::vftables(&bin).unwrap();
    let max_run = cands.iter().map(|(_, n, _)| *n).max().unwrap_or(0);
    assert!(max_run >= 4, "vtable с ~5 указателями: {cands:?}");
}

#[test]
fn decompile_lifts_asm_to_pseudocode() {
    let body = vec![
        "115a:\t55                    \tpush   rbp".to_string(),
        "115b:\t48 89 e5              \tmov    rbp,rsp".to_string(),
        "1162:\t48 89 7d f8           \tmov    QWORD PTR [rbp-0x8],rdi".to_string(),
        "1186:\te8 xx                 \tcall   1119 <net_send>".to_string(),
        "1190:\tc3                    \tret".to_string(),
    ];
    let c = rev::decompile(&body);
    let joined = c.join("\n");
    assert!(joined.contains("rbp = rsp;"), "{joined}");
    assert!(joined.contains("*[rbp-0x8] = rdi;"), "{joined}");
    assert!(joined.contains("net_send(...);"), "{joined}");
    assert!(joined.contains("return;"), "{joined}");
}

#[test]
fn structured_decompiler_finds_if() {
    let body = vec![
        "111b:\t55              \tpush   rbp".to_string(),
        "111c:\t8b 45 f8        \tmov    eax,DWORD PTR [rbp-0x8]".to_string(),
        "111f:\t85 c0           \ttest   eax,eax".to_string(),
        "1121:\t7e 0a           \tjle    112d".to_string(),
        "1123:\tbf 03 00 00 00  \tmov    edi,0x3".to_string(),
        "1128:\te8 xx           \tcall   114a <render_sprite>".to_string(),
        "112d:\tc9              \tleave".to_string(),
        "112e:\tc3              \tret".to_string(),
    ];
    let out = rev::decompile_structured(&body);
    let if_pos = out.find("if (jle) {");
    assert!(if_pos.is_some(), "if найден: {out}");
    let call_pos = out.find("render_sprite");
    assert!(call_pos.is_some() && call_pos.unwrap() > if_pos.unwrap(), "render_sprite внутри if: {out}");
    assert!(out.contains("return;"));
}
