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
//!   emlbox verify <path>
//!   emlbox demo <path> [--big]
//!   emlbox bench <dir>

use emlbox::{bench, demo, fs, ipc, kv, reader, runner, tagdb, verify, writer};
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
        Some("mkdb") => cmd_mkdb(&args[2..]),
        Some("mkmem") => cmd_mkmem(&args[2..]),
        Some("append") => cmd_append(&args[2..]),
        Some("verify") => cmd_verify(&args[2..]),
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
            parts.push(writer::Part::raw(f[0], f[1], f[2], data));
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
                Some(p) => match std::fs::write(&p, data) {
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
                        print!("{}", String::from_utf8_lossy(data));
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
            match kv::set(&path, &table, &key, value) {
                Ok((seq, h)) => {
                    println!("delta seq={seq} hash={h:.12}");
                    0
                }
                Err(e) => err(&e),
            }
        }
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
        _ => err("kv subcommands: get|set|del|dump"),
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
