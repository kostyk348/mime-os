//! emlbox CLI — single-file .eml container toolkit.
//!
//!   emlbox create <path> <entity> <subject> [--part id:ct:name:file ...]
//!   emlbox mount <path>
//!   emlbox get <path> <id> [--out file]
//!   emlbox kv get|set|del|dump <path> <table> [key] [json]
//!   emlbox append <path> <delta-json-file>
//!   emlbox ipc send|list <bus> [<to> <event> [json]]
//!   emlbox run <container> [--bus <dir>] [--once]
//!   emlbox fs index|ls|query|mkdir|dir|tag <store> [...]
//!   emlbox tagdb insert|query|bench <db> [...]
//!   emlbox mkdb <path> <entity>      (X-EML-Type: Database/KV)
//!   emlbox mkmem <path> <entity>     (X-EML-Type: AI/MemoryBank)
//!   emlbox pack <dir> <out.eml> [entity]
//!   emlbox unpack <container> <out-dir>
//!   emlbox verify <path>
//!   emlbox demo <path> [--big]
//!   emlbox bench <dir>

use emlbox::{bench, demo, fs, ipc, kv, mail, pack, reader, repair, rev, runner, site, sync, tagdb, verify, writer};
use serde_json::Value;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let code = match args.get(1).map(|s| s.as_str()) {
        Some("create") => cmd_create(&args[2..]),
        Some("mount") => cmd_mount(&args[2..]),
        Some("get") => cmd_get(&args[2..]),
        Some("kv") => cmd_kv(&args[2..]),
        Some("ipc") => cmd_ipc(&args[2..]),
        Some("run") => cmd_run(&args[2..]),
        Some("fs") => cmd_fs(&args[2..]),
        Some("tagdb") => cmd_tagdb(&args[2..]),
        Some("sync") => cmd_sync(&args[2..]),
        Some("mail") => cmd_mail(&args[2..]),
        Some("rev") => cmd_rev(&args[2..]),
        Some("site") => cmd_site(&args[2..]),
        Some("mkdb") => cmd_mkdb(&args[2..]),
        Some("mkmem") => cmd_mkmem(&args[2..]),
        Some("pack") => cmd_pack(&args[2..]),
        Some("unpack") => cmd_unpack(&args[2..]),
        Some("append") => cmd_append(&args[2..]),
        Some("verify") => cmd_verify(&args[2..]),
        Some("compact") => cmd_compact(&args[2..]),
        Some("repair") => cmd_repair(&args[2..]),
        Some("doc") => cmd_doc(&args[2..]),
        Some("demo") => cmd_demo(&args[2..]),
        Some("bench") => cmd_bench(&args[2..]),
        _ => {
            eprintln!("emlbox: unknown command. See usage in source header.");
            2
        }
    };
    std::process::exit(code);
}

fn path_arg(a: &[String], i: usize, what: &str) -> Result<PathBuf, String> {
    a.get(i)
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing {what}"))
}

fn cmd_create(a: &[String]) -> i32 {
    let path = match path_arg(a, 0, "path") {
        Ok(p) => p,
        Err(e) => return err(&e),
    };
    let entity = a.get(1).cloned().unwrap_or_default();
    let subject = a.get(2).cloned().unwrap_or_default();
    let mut parts = Vec::new();
    let mut i = 3;
    let enc = a
        .iter()
        .position(|x| x == "--enc")
        .and_then(|i| a.get(i + 1))
        .cloned()
        .unwrap_or_else(|| emlbox::format::ENC_RAW.to_string());
    while i < a.len() {
        if a[i] == "--part" {
            let spec = match a.get(i + 1) {
                Some(s) => s.clone(),
                None => return err("--part needs id:ct:name:file"),
            };
            let f: Vec<&str> = spec.splitn(4, ':').collect();
            if f.len() != 4 {
                return err("--part format: id:ct:name:file");
            }
            let data = match std::fs::read(f[3]) {
                Ok(d) => d,
                Err(e) => return err(&format!("read {}: {e}", f[3])),
            };
            parts.push(writer::Part {
                id: f[0].to_string(),
                ct: f[1].to_string(),
                name: f[2].to_string(),
                enc: enc.clone(),
                data,
            });
            i += 2;
        } else if a[i] == "--enc" {
            i += 2;
        } else {
            return err(&format!("unexpected arg {}", a[i]));
        }
    }
    match writer::build_file(&path, &entity, &subject, parts) {
        Ok(()) => {
            println!("created {}", path.display());
            0
        }
        Err(e) => err(&e),
    }
}

fn cmd_mount(a: &[String]) -> i32 {
    let path = match path_arg(a, 0, "path") {
        Ok(p) => p,
        Err(e) => return err(&e),
    };
    match reader::EmlBox::open(&path) {
        Ok(b) => {
            println!("file:  {}", path.display());
            println!("size:  {} B", b.mmap.len());
            println!("entity: {}", b.entity().unwrap_or_default());
            println!("base sections ({}):", b.sections.len());
            for s in &b.sections {
                println!(
                    "  [{:>8}] off={:>10} len={:>9}  {}  {}  ({})",
                    s.id, s.off, s.len, s.ct, s.name, s.enc
                );
            }
            println!("deltas: {} (tail_seq={})", b.tail.entries.len(), b.tail_seq);
            for e in b.tail_entries() {
                println!("  seq={} off={} len={} hash={:.12}", e.seq, e.off, e.len, e.hash);
            }
            println!("base_hash: {}", b.base_hash);
            println!("tail_hash: {}", b.tail_hash);
            0
        }
        Err(e) => err(&e),
    }
}

fn cmd_get(a: &[String]) -> i32 {
    let path = match path_arg(a, 0, "path") {
        Ok(p) => p,
        Err(e) => return err(&e),
    };
    let id = a.get(1).cloned().unwrap_or_default();
    let out = a.iter().position(|x| x == "--out").map(|i| PathBuf::from(a[i + 1].clone()));
    match reader::EmlBox::open(&path) {
        Ok(b) => match b.section(&id) {
            Some(data) => match out {
                Some(p) => match std::fs::write(&p, &data) {
                    Ok(()) => {
                        println!("wrote {} bytes to {}", data.len(), p.display());
                        0
                    }
                    Err(e) => err(&e.to_string()),
                },
                None => {
                    let printable = data
                        .iter()
                        .all(|b| *b == b'\n' || *b == b'\r' || *b == b'\t' || (*b >= 0x20 && *b != 0x7f));
                    if printable {
                        print!("{}", String::from_utf8_lossy(&data));
                    } else {
                        println!("<binary {}, {} bytes>", id, data.len());
                    }
                    0
                }
            },
            None => err(&format!("section '{id}' not found")),
        },
        Err(e) => err(&e),
    }
}

fn cmd_kv(a: &[String]) -> i32 {
    let sub = a.first().map(|s| s.as_str()).unwrap_or("");
    let path = match path_arg(a, 1, "path") {
        Ok(p) => p,
        Err(e) => return err(&e),
    };
    let table = a.get(2).cloned().unwrap_or_default();
    match sub {
        "get" => {
            let key = a.get(3).cloned().unwrap_or_default();
            match reader::EmlBox::open(&path) {
                Ok(b) => match kv::get(&b, &table, &key) {
                    Ok(v) => {
                        println!("{}", v.map(|v| v.to_string()).unwrap_or_else(|| "<missing>".into()));
                        0
                    }
                    Err(e) => err(&e),
                },
                Err(e) => err(&e),
            }
        }
        "set" => {
            let key = a.get(3).cloned().unwrap_or_default();
            let val = a.get(4).cloned().unwrap_or_default();
            let value: Value = match serde_json::from_str(&val) {
                Ok(v) => v,
                Err(e) => return err(&format!("bad json: {e}")),
            };
            let writer = a
                .iter()
                .position(|x| x == "--writer")
                .and_then(|i| a.get(i + 1))
                .cloned()
                .unwrap_or_else(|| "local".to_string());
            match kv::set_w(&path, &writer, &table, &key, value) {
                Ok((seq, h)) => {
                    println!("delta [{writer}] seq={seq} hash={h:.12}");
                    0
                }
                Err(e) => err(&e),
            }
        }
        "add" => {
            let key = a.get(3).cloned().unwrap_or_default();
            let val = a.get(4).cloned().unwrap_or_default();
            let value: Value = match serde_json::from_str(&val) {
                Ok(v) => v,
                Err(e) => return err(&format!("bad json: {e}")),
            };
            let writer = a
                .iter()
                .position(|x| x == "--writer")
                .and_then(|i| a.get(i + 1))
                .cloned()
                .unwrap_or_else(|| "local".to_string());
            let after = a
                .iter()
                .position(|x| x == "--after")
                .and_then(|i| a.get(i + 1))
                .cloned();
            match kv::add(&path, &writer, &table, &key, value, after) {
                Ok((seq, h)) => {
                    println!("add [{writer}] seq={seq} hash={h:.12}");
                    0
                }
                Err(e) => err(&e),
            }
        }
        "list" => match reader::EmlBox::open(&path) {
            Ok(b) => {
                let key = a.get(3).cloned().unwrap_or_default();
                match kv::list(&b, &table, &key) {
                Ok(items) => {
                    for it in &items {
                        println!("{}", it);
                    }
                    0
                }
                Err(e) => err(&e),
                }
            },
            Err(e) => err(&e),
        },
        "del" => {
            let key = a.get(3).cloned().unwrap_or_default();
            match kv::del(&path, &table, &key) {
                Ok((seq, h)) => {
                    println!("delta seq={seq} hash={h:.12}");
                    0
                }
                Err(e) => err(&e),
            }
        }
        "dump" => match reader::EmlBox::open(&path) {
            Ok(b) => match kv::table(&b, &table) {
                Ok(v) => {
                    println!("{}", serde_json::to_string_pretty(&v).unwrap());
                    0
                }
                Err(e) => err(&e),
            },
            Err(e) => err(&e),
        },
        _ => err("kv subcommands: get|set|add|list|del|dump"),
    }
}

fn cmd_ipc(a: &[String]) -> i32 {
    let sub = a.first().map(|s| s.as_str()).unwrap_or("");
    match sub {
        "send" => {
            let bus = match path_arg(a, 1, "bus") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let to = a.get(2).cloned().unwrap_or_default();
            let event = a.get(3).cloned().unwrap_or_default();
            let body: Value = a.get(4).map(|s| serde_json::from_str(s).unwrap_or_else(|_| Value::Object(serde_json::Map::new()))).unwrap_or_else(|| Value::Object(serde_json::Map::new()));
            match ipc::send(&bus, "view", &to, &event, &body) {
                Ok(p) => {
                    println!("sent {}", p.display());
                    0
                }
                Err(e) => err(&e),
            }
        }
        "list" => {
            let bus = match path_arg(a, 1, "bus") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            match ipc::list(&bus) {
                Ok(msgs) => {
                    if msgs.is_empty() {
                        println!("bus empty");
                    }
                    for m in &msgs {
                        match ipc::parse(m) {
                            Ok(msg) => println!(
                                "{}  {} -> {}  [{}]  {}",
                                msg.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                                msg.from.trim_matches(|c| c == '<' || c == '>'),
                                msg.to.trim_matches(|c| c == '<' || c == '>'),
                                msg.event,
                                msg.body
                            ),
                            Err(e) => println!("{}: {e}", m.display()),
                        }
                    }
                    0
                }
                Err(e) => err(&e),
            }
        }
        _ => err("ipc subcommands: send|list"),
    }
}

fn cmd_run(a: &[String]) -> i32 {
    let path = match path_arg(a, 0, "container") {
        Ok(p) => p,
        Err(e) => return err(&e),
    };
    let bus = a
        .iter()
        .position(|x| x == "--bus")
        .map(|i| PathBuf::from(a[i + 1].clone()))
        .unwrap_or_else(|| PathBuf::from("/dev/shm/emlbox_bus"));
    let once = a.iter().any(|x| x == "--once");
    match runner::run(&path, &bus, once, 10) {
        Ok(n) => {
            println!("processed {n} event(s)");
            0
        }
        Err(e) => err(&e),
    }
}

fn cmd_fs(a: &[String]) -> i32 {
    let sub = a.first().map(|s| s.as_str()).unwrap_or("");
    let store = match path_arg(a, 1, "store") {
        Ok(p) => p,
        Err(e) => return err(&e),
    };
    match sub {
        "index" | "ls" => match fs::index(&store) {
            Ok((recs, skipped)) => {
                for r in &recs {
                    println!(
                        "  {:<32} {:>20}  [{:>7}]  tags={}  {}",
                        r.entity,
                        r.eml_type,
                        r.size,
                        r.tags.join(","),
                        r.subject
                    );
                }
                println!("{} container(s), {} skipped", recs.len(), skipped.len());
                0
            }
            Err(e) => err(&e),
        },
        "query" => {
            let q = match a.get(2) {
                Some(q) => q.clone(),
                None => return err("query: need X-Query string"),
            };
            let (recs, _) = match fs::index(&store) {
                Ok(x) => x,
                Err(e) => return err(&e),
            };
            match fs::eval(&recs, &q) {
                Ok(matches) => {
                    for r in matches {
                        println!("  {}  ({})", r.entity, r.eml_type);
                    }
                    0
                }
                Err(e) => err(&e),
            }
        }
        "mkdir" => {
            let name = match a.get(2) {
                Some(n) => n.clone(),
                None => return err("mkdir: need directory name"),
            };
            let query = a.iter().position(|x| x == "--query").map(|i| a[i + 1].clone());
            let contains: Vec<String> = {
                let mut v = Vec::new();
                let mut i = 3;
                while i < a.len() {
                    if a[i] == "--contains" {
                        if let Some(id) = a.get(i + 1) {
                            v.push(id.clone());
                            i += 2;
                            continue;
                        }
                    }
                    i += 1;
                }
                v
            };
            match fs::mkdir(&store, &name, query.as_deref(), &contains) {
                Ok(p) => {
                    println!("created {}", p.display());
                    0
                }
                Err(e) => err(&e),
            }
        }
        "dir" => {
            let name = match a.get(2) {
                Some(n) => n.clone(),
                None => return err("dir: need directory name"),
            };
            match fs::index(&store) {
                Ok((recs, _)) => match recs.iter().find(|r| r.entity == format!("{name}@system.local")) {
                    Some(dir) => {
                        for r in fs::resolve(&recs, dir) {
                            println!("  {}  ({})", r.entity, r.eml_type);
                        }
                        0
                    }
                    None => err(&format!("directory {name}@system.local not found")),
                },
                Err(e) => err(&e),
            }
        }
        "tag" => {
            let entity = match a.get(2) {
                Some(e) => e.clone(),
                None => return err("tag: need entity"),
            };
            let tag = match a.get(3) {
                Some(t) => t.clone(),
                None => return err("tag: need tag"),
            };
            match fs::tag(&store, &entity, &tag) {
                Ok((seq, h)) => {
                    println!("tagged {entity} with '{tag}' (delta seq={seq}, {h:.8})");
                    0
                }
                Err(e) => err(&e),
            }
        }
        _ => err("fs subcommands: index|ls|query|mkdir|dir|tag"),
    }
}

fn cmd_tagdb(a: &[String]) -> i32 {
    let sub = a.first().map(|s| s.as_str()).unwrap_or("");
    let db = match path_arg(a, 1, "db") {
        Ok(p) => p,
        Err(e) => return err(&e),
    };
    match sub {
        "insert" => {
            let json = match a.get(2) {
                Some(j) => j.clone(),
                None => return err("insert: need JSON body"),
            };
            let body: Value = match serde_json::from_str(&json) {
                Ok(v) => v,
                Err(e) => return err(&format!("bad json: {e}")),
            };
            let mut id = String::new();
            let mut tags = Vec::new();
            let mut device = String::new();
            let mut ts = 0u64;
            let mut i = 3;
            while i < a.len() {
                match a[i].as_str() {
                    "--id" => {
                        id = a.get(i + 1).cloned().unwrap_or_default();
                        i += 2;
                    }
                    "--tag" => {
                        if let Some(t) = a.get(i + 1) {
                            tags.push(t.clone());
                        }
                        i += 2;
                    }
                    "--device" => {
                        device = a.get(i + 1).cloned().unwrap_or_default();
                        i += 2;
                    }
                    "--ts" => {
                        ts = a.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(0);
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
            if id.is_empty() {
                id = format!(
                    "rec_{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos())
                        .unwrap_or(0)
                );
            }
            match tagdb::insert(&db, &id, &tags, &device, ts, &body) {
                Ok(p) => {
                    println!("inserted {}", p.display());
                    0
                }
                Err(e) => err(&e),
            }
        }
        "query" => {
            let q = match a.get(2) {
                Some(q) => q.clone(),
                None => return err("query: need X-Query"),
            };
            match tagdb::query(&db, &q) {
                Ok((matches, corrupt)) => {
                    for (p, h) in &matches {
                        let rid = h
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("X-Record-ID"))
                            .map(|(_, v)| v.clone())
                            .unwrap_or_default();
                        println!("  {}  ({rid})", p.display());
                    }
                    println!("{} match(es), {corrupt} corrupt skipped", matches.len());
                    0
                }
                Err(e) => err(&e),
            }
        }
        "bench" => {
            let n = a.iter().position(|x| x == "--n").and_then(|i| a.get(i + 1)).and_then(|s| s.parse().ok()).unwrap_or(5000);
            let body_kb = a.iter().position(|x| x == "--body-kb").and_then(|i| a.get(i + 1)).and_then(|s| s.parse().ok()).unwrap_or(2);
            match tagdb::bench(&db, n, body_kb) {
                Ok(out) => {
                    println!("{out}");
                    0
                }
                Err(e) => err(&e),
            }
        }
        _ => err("tagdb subcommands: insert|query|bench"),
    }
}

fn cmd_rev(a: &[String]) -> i32 {
    let sub = a.first().map(|s| s.as_str()).unwrap_or("");
    let _flag = |name: &str| a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned());
    match sub {
        "type" => {
            // rev type <dir> <func> arg0 <Type>  — пометить тип (depth 0)
            let dir = match path_arg(a, 1, "dir") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let func = a.get(2).cloned().unwrap_or_default();
            let arg = a.get(3).cloned().unwrap_or_default();
            let ty = a.get(4).cloned().unwrap_or_default();
            match rev::type_mark(&dir, &func, &arg, &ty, 0) {
                Ok(hit) => {
                    for f in &hit {
                        println!("  [{f}] {arg}: {ty}");
                    }
                    0
                }
                Err(e) => err(&e),
            }
        }
        "wave" => {
            // rev wave <dir> <func> arg0 <Type> <ticks>  — волна на N тактов
            let dir = match path_arg(a, 1, "dir") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let func = a.get(2).cloned().unwrap_or_default();
            let arg = a.get(3).cloned().unwrap_or_default();
            let ty = a.get(4).cloned().unwrap_or_default();
            let ticks: usize = a.get(5).and_then(|s| s.parse().ok()).unwrap_or(3);
            match rev::type_mark(&dir, &func, &arg, &ty, ticks) {
                Ok(hit) => {
                    println!("волна за {ticks} тактов достигла {} функций:", hit.len());
                    for f in &hit {
                        println!("  {f}");
                    }
                    0
                }
                Err(e) => err(&e),
            }
        }
        "cluster" => {
            // rev cluster <dir> <pattern>
            let dir = match path_arg(a, 1, "dir") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let pat = a.get(2).cloned().unwrap_or_default();
            match rev::cluster(&dir, &pat) {
                Ok(hits) => {
                    println!("{} совпадений по '{pat}':", hits.len());
                    for h in &hits {
                        println!("  {h}");
                    }
                    0
                }
                Err(e) => err(&e),
            }
        }
        "graph" => {
            let dir = match path_arg(a, 1, "dir") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            match rev::graph(&dir) {
                Ok(g) => {
                    for (name, callees) in &g {
                        println!("{name} -> {}", callees.join(", "));
                    }
                    0
                }
                Err(e) => err(&e),
            }
        }
        "types" => {
            let dir = match path_arg(a, 1, "dir") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            match rev::type_map(&dir) {
                Ok(t) => {
                    if t.is_empty() {
                        println!("типов пока нет — rev type <dir> <func> arg0 <Type>");
                    }
                    for (name, types) in &t {
                        println!("{name}: {}", types.join(", "));
                    }
                    0
                }
                Err(e) => err(&e),
            }
        }
        "hash" => {
            let dir = match path_arg(a, 1, "dir") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let func = a.get(2).cloned().unwrap_or_default();
            match rev::body_hash(&dir, &func) {
                Ok(h) => {
                    println!("{func}: {h}");
                    0
                }
                Err(e) => err(&e),
            }
        }
        "branch" => {
            // rev branch <dir> <func> <name> [--depth N]
            let dir = match path_arg(a, 1, "dir") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let func = a.get(2).cloned().unwrap_or_default();
            let name = a.get(3).cloned().unwrap_or_default();
            if name == "rm" {
                let br = a.get(4).cloned().unwrap_or_default();
                match rev::branch_rm(&dir, &br) {
                    Ok(n) => {
                        println!("ветка {br} удалена ({n} клеток)");
                        0
                    }
                    Err(e) => err(&e),
                }
            } else if name == "list" {
                match rev::branches(&dir) {
                    Ok(list) => {
                        for (br, root, n) in &list {
                            println!("  {br}: корень {root}, {n} клеток");
                        }
                        if list.is_empty() {
                            println!("веток нет");
                        }
                        0
                    }
                    Err(e) => err(&e),
                }
            } else {
                let depth: usize = a
                    .iter()
                    .position(|x| x == "--depth")
                    .and_then(|i| a.get(i + 1))
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(2);
                match rev::branch(&dir, &func, &name, depth) {
                    Ok(n) => {
                        println!("ветка {name} от {func}: {n} клеток (depth {depth})");
                        println!("волна: rev wave <dir> {func}@{name} arg0 Type ticks");
                        0
                    }
                    Err(e) => err(&e),
                }
            }
        }
        "diff" => {
            // rev diff <dirA> <dirB>
            let dir_a = match path_arg(a, 1, "dirA") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let dir_b = match path_arg(a, 2, "dirB") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            match rev::diff(&dir_a, &dir_b) {
                Ok((changed, added, removed)) => {
                    println!("changed ({}): {}", changed.len(), changed.join(", "));
                    println!("added ({}): {}", added.len(), added.join(", "));
                    println!("removed ({}): {}", removed.len(), removed.join(", "));
                    0
                }
                Err(e) => err(&e),
            }
        }
        // emlbox rev <binary> <dir>  — полный конвейер: objdump -> .eml-граф
        _ => {
            let binary = match path_arg(a, 0, "binary") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let dir = match path_arg(a, 1, "dir") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            match rev::analyze(&binary, &dir) {
                Ok(n) => {
                    println!("{n} функций -> {}", dir.display());
                    match rev::graph(&dir) {
                        Ok(g) => {
                            for (name, callees) in &g {
                                println!("  {name} -> {}", callees.join(", "));
                            }
                        }
                        Err(e) => return err(&e),
                    }
                    0
                }
                Err(e) => err(&e),
            }
        }
    }
}

fn cmd_mail(a: &[String]) -> i32 {
    let sub = a.first().map(|s| s.as_str()).unwrap_or("");
    let flag = |name: &str| a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned());
    match sub {
        "pack" => {
            // mail pack <container> --to addr [--writer W] [--since N] [--out file]
            let container = match path_arg(a, 1, "container") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let to = flag("--to").unwrap_or_default();
            let writer = flag("--writer").unwrap_or_else(|| "local".to_string());
            let since: u64 = flag("--since").and_then(|s| s.parse().ok()).unwrap_or(0);
            match mail::pack(&container, &writer, &to, since) {
                Ok(bytes) => {
                    let out = flag("--out").map(PathBuf::from).unwrap_or_else(|| {
                        let p = PathBuf::from("outbox");
                        let _ = std::fs::create_dir_all(&p);
                        p.join(format!("sync_{}.eml", since))
                    });
                    match std::fs::write(&out, &bytes) {
                        Ok(()) => {
                            println!("picked {} bytes -> {}", bytes.len(), out.display());
                            println!("отправь это письмо любым SMTP-клиентом; приём — mail receive из Maildir");
                            0
                        }
                        Err(e) => err(&format!("write {}: {e}", out.display())),
                    }
                }
                Err(e) => err(&e),
            }
        }
        "apply" => {
            // mail apply <letter.eml> <container>
            let letter = match path_arg(a, 1, "letter") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let container = match path_arg(a, 2, "container") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            match std::fs::read(&letter) {
                Ok(bytes) => match mail::apply(&container, &bytes) {
                    Ok((applied, pending)) => {
                        println!("applied {applied}, pending {pending}");
                        0
                    }
                    Err(e) => err(&e),
                },
                Err(e) => err(&format!("read {}: {e}", letter.display())),
            }
        }
        "receive" => {
            // mail receive <maildir> <container>
            let maildir = match path_arg(a, 1, "maildir") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let container = match path_arg(a, 2, "container") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            match mail::receive(&container, &maildir) {
                Ok((applied, pending)) => {
                    println!("applied {applied}, pending {pending}");
                    0
                }
                Err(e) => err(&e),
            }
        }
        _ => err("mail subcommands: pack|apply|receive"),
    }
}

fn cmd_sync(a: &[String]) -> i32 {
    let sub = a.first().map(|s| s.as_str()).unwrap_or("");
    let flag = |name: &str| a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned());
    match sub {
        "export" => {
            let container = match path_arg(a, 1, "container") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let writer = flag("--writer").unwrap_or_else(|| "local".to_string());
            let since: u64 = flag("--since").and_then(|s| s.parse().ok()).unwrap_or(0);
            match sync::export(&container, &writer, since) {
                Ok(blocks) => {
                    for (seq, b) in &blocks {
                        println!(
                            "writer {writer} seq={seq} ({} bytes, {})",
                            b.len(),
                            &emlbox::format::hash_bytes(b)[..12]
                        );
                    }
                    println!("{} block(s), seq > {since}", blocks.len());
                    0
                }
                Err(e) => err(&e),
            }
        }
        "push" => {
            let container = match path_arg(a, 1, "container") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let writer = flag("--writer").unwrap_or_else(|| "local".to_string());
            let bus = flag("--bus").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/dev/shm/emlbox_bus"));
            let to = flag("--to").unwrap_or_else(|| "*".to_string());
            let since: u64 = flag("--since").and_then(|s| s.parse().ok()).unwrap_or(0);
            match sync::push(&container, &writer, &bus, &to, since) {
                Ok(n) => {
                    println!("pushed {n} block(s) of writer {writer} to {}", bus.display());
                    0
                }
                Err(e) => err(&e),
            }
        }
        "pull" => {
            let container = match path_arg(a, 1, "container") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let bus = flag("--bus").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/dev/shm/emlbox_bus"));
            match sync::pull(&container, &bus) {
                Ok((applied, pending)) => {
                    println!("applied {applied}, pending {pending}");
                    0
                }
                Err(e) => err(&e),
            }
        }
        "apply" => {
            let container = match path_arg(a, 1, "container") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let block_file = match path_arg(a, 2, "block file") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let block = match std::fs::read(&block_file) {
                Ok(b) => b,
                Err(e) => return err(&format!("read {}: {e}", block_file.display())),
            };
            match sync::apply_block(&container, &block) {
                Ok((w, seq)) => {
                    println!("applied {w}#{seq}");
                    0
                }
                Err(e) => err(&e),
            }
        }
        "heads" => {
            let container = match path_arg(a, 1, "container") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            match sync::heads(&container) {
                Ok(h) => {
                    for (w, seq, hash) in &h {
                        println!("  {w}: seq={seq} hash={hash}");
                    }
                    println!("{} writer chain(s)", h.len());
                    0
                }
                Err(e) => err(&e),
            }
        }
        "serve" => {
            let container = match path_arg(a, 1, "container") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let addr = flag("--addr").unwrap_or_else(|| "127.0.0.1:9001".to_string());
            match sync::tcp_serve(&container, &addr) {
                Ok(()) => 0,
                Err(e) => err(&e),
            }
        }
        "connect" => {
            let container = match path_arg(a, 1, "container") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let peer = flag("--peer").unwrap_or_else(|| "127.0.0.1:9001".to_string());
            match sync::tcp_connect(&container, &peer) {
                Ok((recv, sent)) => {
                    println!("sync done: received {recv}, sent {sent}");
                    0
                }
                Err(e) => err(&e),
            }
        }
        _ => err("sync subcommands: export|push|pull|apply|heads|serve|connect"),
    }
}

fn cmd_mkdb(a: &[String]) -> i32 {
    let path = match path_arg(a, 0, "path") {
        Ok(p) => p,
        Err(e) => return err(&e),
    };
    let entity = a.get(1).cloned().unwrap_or_else(|| "db_storage@system.local".to_string());
    match demo::build_db(&path, &entity) {
        Ok(()) => {
            println!("Database/KV built: {}", path.display());
            0
        }
        Err(e) => err(&e),
    }
}

fn cmd_mkmem(a: &[String]) -> i32 {
    let path = match path_arg(a, 0, "path") {
        Ok(p) => p,
        Err(e) => return err(&e),
    };
    let entity = a.get(1).cloned().unwrap_or_else(|| "memory_agent_01@system.local".to_string());
    match demo::build_mem(&path, &entity) {
        Ok(()) => {
            println!("AI/MemoryBank built: {}", path.display());
            0
        }
        Err(e) => err(&e),
    }
}

fn cmd_pack(a: &[String]) -> i32 {
    let dir = match path_arg(a, 0, "dir") {
        Ok(p) => p,
        Err(e) => return err(&e),
    };
    let out = match path_arg(a, 1, "out.eml") {
        Ok(p) => p,
        Err(e) => return err(&e),
    };
    let entity = a
        .get(2)
        .cloned()
        .unwrap_or_else(|| "pack@system.local".to_string());
    match pack::pack(&dir, &out, &entity) {
        Ok((n, bytes)) => {
            println!("packed {n} file(s), {bytes} bytes -> {}", out.display());
            0
        }
        Err(e) => err(&e),
    }
}

fn cmd_unpack(a: &[String]) -> i32 {
    let container = match path_arg(a, 0, "container") {
        Ok(p) => p,
        Err(e) => return err(&e),
    };
    let out = match path_arg(a, 1, "out-dir") {
        Ok(p) => p,
        Err(e) => return err(&e),
    };
    match pack::unpack(&container, &out) {
        Ok(n) => {
            println!("unpacked {n} file(s) -> {}", out.display());
            0
        }
        Err(e) => err(&e),
    }
}

fn cmd_append(a: &[String]) -> i32 {
    let path = match path_arg(a, 0, "path") {
        Ok(p) => p,
        Err(e) => return err(&e),
    };
    let dfile = match path_arg(a, 1, "delta file") {
        Ok(p) => p,
        Err(e) => return err(&e),
    };
    let data = match std::fs::read(&dfile) {
        Ok(d) => d,
        Err(e) => return err(&format!("read {}: {e}", dfile.display())),
    };
    match writer::append_block(&path, &data) {
        Ok((seq, h)) => {
            println!("appended seq={seq} hash={h:.12}");
            0
        }
        Err(e) => err(&e),
    }
}

fn cmd_verify(a: &[String]) -> i32 {
    let path = match path_arg(a, 0, "path") {
        Ok(p) => p,
        Err(e) => return err(&e),
    };
    match verify::verify(&path) {
        Ok(issues) => {
            if issues.is_empty() {
                println!("OK: offsets, base hash, and delta chain verified");
                0
            } else {
                for i in &issues {
                    println!("BROKEN: {i}");
                }
                1
            }
        }
        Err(e) => err(&e),
    }
}

fn cmd_demo(a: &[String]) -> i32 {
    let path = match path_arg(a, 0, "path") {
        Ok(p) => p,
        Err(e) => return err(&e),
    };
    let big = a.iter().any(|x| x == "--big");
    match demo::build_demo(&path, big) {
        Ok(()) => {
            println!("demo built: {}", path.display());
            0
        }
        Err(e) => err(&e),
    }
}

fn cmd_bench(a: &[String]) -> i32 {
    let dir = match path_arg(a, 0, "dir") {
        Ok(p) => p,
        Err(e) => return err(&e),
    };
    match bench::run_all(&dir) {
        Ok(out) => {
            println!("{out}");
            0
        }
        Err(e) => err(&e),
    }
}

fn err(e: &str) -> i32 {
    eprintln!("emlbox: {e}");
    1
}

fn cmd_site(a: &[String]) -> i32 {
    let sub = a.first().map(|s| s.as_str()).unwrap_or("");
    let flag = |name: &str| a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned());
    match sub {
        "new" => {
            // site new <file.eml> --title T [--tags a,b] [--src body.md]
            let file = match path_arg(a, 1, "post file") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let title = flag("--title").unwrap_or_else(|| "Untitled".to_string());
            let tags: Vec<String> = flag("--tags")
                .map(|t| t.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
                .unwrap_or_default();
            let body = match flag("--src") {
                Some(src) => match std::fs::read_to_string(&src) {
                    Ok(b) => b,
                    Err(e) => return err(&format!("read {src}: {e}")),
                },
                None => String::new(),
            };
            match site::new_post(&file, &title, &tags, &body) {
                Ok(()) => {
                    println!("post created: {}", file.display());
                    0
                }
                Err(e) => err(&e),
            }
        }
        "hugo" => {
            // site hugo <posts-dir> <hugo-content-dir>
            let posts = match path_arg(a, 1, "posts dir") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let content = match path_arg(a, 2, "hugo content dir") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            match site::export_hugo(&posts, &content) {
                Ok(n) => {
                    println!("exported {n} постов -> {}/posts/", content.display());
                    0
                }
                Err(e) => err(&e),
            }
        }
        _ => {
            // site <posts-dir> <out-dir>
            let posts = match path_arg(a, 0, "posts dir") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let out = match path_arg(a, 1, "out dir") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            match site::build(&posts, &out) {
                Ok(n) => {
                    println!("site built: {n} постов -> {}", out.display());
                    0
                }
                Err(e) => err(&e),
            }
        }
    }
}

fn cmd_compact(a: &[String]) -> i32 {
    // compact <container> [--out new.eml]
    let src = match path_arg(a, 0, "container") {
        Ok(p) => p,
        Err(e) => return err(&e),
    };
    let out = a
        .iter()
        .position(|x| x == "--out")
        .and_then(|i| a.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("{}.compacted.eml", src.display())));
    match emlbox::compact::compact(&src, &out) {
        Ok((sections, deltas)) => {
            println!("compact: {deltas} дельт слито в {sections} секций -> {}", out.display());
            0
        }
        Err(e) => err(&e),
    }
}

fn cmd_repair(a: &[String]) -> i32 {
    let path = match path_arg(a, 0, "container") {
        Ok(p) => p,
        Err(e) => return err(&e),
    };
    match repair::repair(&path) {
        Ok((blocks, removed)) => {
            println!("repair: восстановлено {blocks} блоков, отброшено {removed} байт -> {}", path.display());
            0
        }
        Err(e) => err(&e),
    }
}

fn cmd_doc(a: &[String]) -> i32 {
    let sub = a.first().map(|s| s.as_str()).unwrap_or("");
    let flag = |name: &str| a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned());
    match sub {
        "init" => {
            // doc init <file> [entity]
            let file = match path_arg(a, 1, "file") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let entity = a.get(2).cloned().unwrap_or_else(|| "doc@system.local".to_string());
            match writer::build_file(&file, &entity, "doc", vec![writer::Part::raw("doc", "application/json", "doc.json", br#"{"lines":[]}"#.to_vec())]) {
                Ok(()) => {
                    println!("doc created: {}", file.display());
                    0
                }
                Err(e) => err(&e),
            }
        }
        "add" => {
            // doc add <file> <text> [--writer W] [--after id]
            let file = match path_arg(a, 1, "file") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let text = a.get(2).cloned().unwrap_or_default();
            let writer = flag("--writer").unwrap_or_else(|| "local".to_string());
            let after = flag("--after");
            match kv::add(&file, &writer, "doc", "lines", serde_json::Value::String(text), after) {
                Ok((seq, h)) => {
                    println!("doc line [{writer}] seq={seq} hash={h:.12}");
                    0
                }
                Err(e) => err(&e),
            }
        }
        "list" => {
            // doc list <file> [-v]
            let file = match path_arg(a, 1, "file") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let verbose = a.iter().any(|x| x == "-v");
            match reader::EmlBox::open(&file) {
                Ok(b) => {
                    let t = match kv::table(&b, "doc") {
                        Ok(t) => t,
                        Err(e) => return err(&e),
                    };
                    match t.get("lines") {
                        Some(serde_json::Value::Array(arr)) => {
                            for e in arr {
                                if verbose {
                                    let id = e.get("id").and_then(|i| i.as_str()).unwrap_or("?");
                                    let v = e.get("v").and_then(|v| v.as_str()).unwrap_or("");
                                    println!("[{id}] {v}");
                                } else {
                                    println!("{}", e.get("v").and_then(|v| v.as_str()).unwrap_or(""));
                                }
                            }
                            0
                        }
                        _ => err("doc: no lines"),
                    }
                }
                Err(e) => err(&e),
            }
        }
        "set" => {
            // doc set <file> <text> --id <id> [--writer W]
            let file = match path_arg(a, 1, "file") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let text = a.get(2).cloned().unwrap_or_default();
            let id = flag("--id").unwrap_or_default();
            let writer = flag("--writer").unwrap_or_else(|| "local".to_string());
            match kv::list_set(&file, &writer, "doc", "lines", &id, serde_json::Value::String(text)) {
                Ok((seq, h)) => {
                    println!("doc line [{writer}] set {id} seq={seq} hash={h:.12}");
                    0
                }
                Err(e) => err(&e),
            }
        }
        "del" => {
            // doc del <file> --id <id> [--writer W]
            let file = match path_arg(a, 1, "file") {
                Ok(p) => p,
                Err(e) => return err(&e),
            };
            let id = flag("--id").unwrap_or_default();
            let writer = flag("--writer").unwrap_or_else(|| "local".to_string());
            match kv::list_del(&file, &writer, "doc", "lines", &id) {
                Ok((seq, h)) => {
                    println!("doc line [{writer}] del {id} seq={seq} hash={h:.12}");
                    0
                }
                Err(e) => err(&e),
            }
        }
        _ => err("doc subcommands: init|add|set|del|list"),
    }
}
