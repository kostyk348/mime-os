//! Клеточный реверс (prototype): бинарник -> граф .eml-функций.
//!
//! Каждая функция дизассемблированного листинга становится отдельным .eml:
//!   From: <name>@binary.target
//!   To:   <callee1>, <callee2>, ...      (call-граф, кто вызывается)
//!   References: <caller1>, ...           (XREFs, кто вызывает — computed)
//!   X-Type-Arg0: Player*                 (волновое распространение типов)
//!   Body: listing секция (assembly)
//!
//! Волна типов: пометили тип аргумента в одной клетке -> BFS по References
//! распространяет метку на вызывающие функции (они держат тот же указатель,
//! чтобы передать его в аргумент). Ограничение прототипа: без анализа позиции
//! аргумента на call-site — метка растекается как "указатель того же типа".

use crate::format::hash_bytes;
use crate::reader::EmlBox;
use crate::writer::{build_file_with_headers, Part};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

pub const BINARY: &str = "binary.target";

#[derive(Clone, Debug)]
pub struct RevFunc {
    pub name: String,
    pub callees: Vec<String>,
    pub body: Vec<String>,
}

/// Полный конвейер: objdump -d бинарника -> каталог .eml-функций.
/// Возвращает число функций.
pub fn analyze(binary: &Path, dir: &Path) -> Result<usize, String> {
    let listing = objdump_listing(binary)?;
    build(dir, &listing)
}

fn objdump_listing(binary: &Path) -> Result<String, String> {
    let run = |args: &[&str]| {
        std::process::Command::new("objdump")
            .args(args)
            .arg(binary)
            .output()
            .map_err(|e| format!("objdump: {e}"))
    };
    let intel = run(&["-d", "-M", "intel"])?;
    if intel.status.success() {
        return Ok(String::from_utf8_lossy(&intel.stdout).to_string());
    }
    let plain = run(&["-d"])?;
    if plain.status.success() {
        return Ok(String::from_utf8_lossy(&plain.stdout).to_string());
    }
    Err(format!("objdump failed: {}", String::from_utf8_lossy(&intel.stderr)))
}

/// Парсинг objdump -d -M intel листинга.
pub fn parse_listing(listing: &str) -> Vec<RevFunc> {
    // строки вида "0000000000401136 <main>:" открывают функцию
    let mut funcs: Vec<RevFunc> = Vec::new();
    let mut cur: Option<RevFunc> = None;
    for line in listing.lines() {
        let trimmed = line.trim();
        if let Some(open) = trimmed.find(" <") {
            if trimmed.ends_with(">:") || trimmed.ends_with('>') && trimmed.contains(">:") {
                let name = trimmed[open + 2..trimmed.len() - 2].to_string();
                if let Some(f) = cur.take() {
                    funcs.push(f);
                }
                cur = Some(RevFunc { name, callees: Vec::new(), body: Vec::new() });
                continue;
            }
        }
        if let Some(f) = cur.as_mut() {
            // извлечь имена из call-инструкций: "call 401090 <net_send@plt>"
            if let Some(ci) = trimmed.find("call") {
                let rest = &trimmed[ci + 4..];
                if let Some(l) = rest.find('<') {
                    if let Some(r) = rest[l + 1..].find('>') {
                        let name = rest[l + 1..l + 1 + r].to_string();
                        if !f.callees.contains(&name) {
                            f.callees.push(name);
                        }
                    }
                }
            }
            f.body.push(trimmed.to_string());
        }
    }
    if let Some(f) = cur.take() {
        funcs.push(f);
    }
    funcs
}

fn safe_name(name: &str) -> String {
    name.chars().map(|c| if c == '/' || c == '\\' { '_' } else { c }).collect()
}

fn func_file(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{}.eml", safe_name(name)))
}

fn to_header(list: &[String]) -> String {
    if list.is_empty() { "".into() } else { format!("X-Callees: {}\r\n", list.join(", ")) }
}

/// Создать каталог .eml-функций из листинга. Returns число функций.
pub fn build(dir: &Path, listing: &str) -> Result<usize, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let funcs = parse_listing(listing);
    // References: кто вызывает кого (обратный граф)
    let mut callers: HashMap<String, Vec<String>> = HashMap::new();
    for f in &funcs {
        for c in &f.callees {
            callers.entry(c.clone()).or_default().push(f.name.clone());
        }
    }
    for f in &funcs {
        let refs = callers.get(&f.name).cloned().unwrap_or_default();
        let mut extra = String::new();
        extra.push_str(&format!("X-EML-Type: Reverse/Binary-Function\r\n"));
        extra.push_str(&to_header(&f.callees));
        if !refs.is_empty() {
            extra.push_str(&format!("References: {}\r\n", refs.join(", ")));
        }
        extra.push_str(&format!("X-Func-Lines: {}\r\n", f.body.len()));
        let body = f.body.join("\n");
        let entity = format!("{}@{}", safe_name(&f.name), BINARY);
        build_file_with_headers(
            &func_file(dir, &f.name),
            &entity,
            &f.name,
            &extra,
            vec![Part::raw("listing", "text/x-asm", "listing.txt", body.into_bytes())],
        )
        .map_err(|e| format!("{}: {e}", f.name))?;
    }
    Ok(funcs.len())
}

/// Кто вызывает `name` (References) — читается из .eml заголовков.
pub fn callers(dir: &Path, name: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let rd = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    for e in rd.flatten() {
        let p = e.path();
        if !p.extension().map(|x| x == "eml").unwrap_or(false) {
            continue;
        }
        let b = EmlBox::open(&p)?;
        let mut is_caller = false;
        for (k, v) in &b.headers {
            if k.eq_ignore_ascii_case("X-Callees") {
                for t in v.split(',') {
                    if t.trim() == name {
                        is_caller = true;
                    }
                }
            }
        }
        if is_caller {
            if let Some(en) = b.entity() {
                let en = en.trim_end_matches(&format!("@{}", BINARY));
                out.push(en.to_string());
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Волновое распространение типов: пометить `func` типом `ty` для аргумента
/// `arg` (например "arg0"), затем BFS по References (вызывающие функции —
/// они держат тот же указатель) на `max_depth` тактов. depth=0 — только
/// сама функция. Возвращает функции, до которых дошла волна.
pub fn type_mark(dir: &Path, func: &str, arg: &str, ty: &str, max_depth: usize) -> Result<Vec<String>, String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue = VecDeque::new();
    let mut reached = Vec::new();
    queue.push_back((func.to_string(), 0usize));
    seen.insert(func.to_string());
    while let Some((name, depth)) = queue.pop_front() {
        let p = func_file(dir, &name);
        if !p.exists() {
            continue;
        }
        // прочитать текущие X-Type-* заголовки
        let b = EmlBox::open(&p)?;
        let mut types: Vec<(String, String)> = Vec::new();
        for (k, v) in &b.headers {
            let k = k.to_string();
            if k.starts_with("X-Type-") {
                types.push((k, v.clone()));
            }
        }
        let tkey = format!("X-Type-{arg}");
        if !types.iter().any(|(k, _)| k == &tkey) {
            types.push((tkey.clone(), ty.to_string()));
        }
        reached.push(name.clone());
        // пересоздать файл с новыми заголовками
        rewrite(dir, &name, &types)?;
        if depth >= max_depth {
            continue;
        }
        // распространить вверх: кто вызывает name
        let callers = callers(dir, &name)?;
        for c in callers {
            if seen.insert(c.clone()) {
                queue.push_back((c, depth + 1));
            }
        }
    }
    Ok(reached)
}

fn rewrite(dir: &Path, name: &str, types: &[(String, String)]) -> Result<(), String> {
    let p = func_file(dir, name);
    let b = EmlBox::open(&p)?;
    let listing = b
        .section("listing")
        .ok_or("no listing section")?;
    let callees: Vec<String> = b
        .headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("X-Callees"))
        .flat_map(|(_, v)| v.split(',').map(|s| s.trim().to_string()))
        .collect();
    let refs: Vec<String> = callers(dir, name)?;
    let mut extra = String::new();
    extra.push_str("X-EML-Type: Reverse/Binary-Function\r\n");
    extra.push_str(&to_header(&callees));
    if !refs.is_empty() {
        extra.push_str(&format!("References: {}\r\n", refs.join(", ")));
    }
    for (k, v) in types {
        extra.push_str(&format!("{k}: {v}\r\n"));
    }
    let entity = format!("{}@{}", safe_name(name), BINARY);
    build_file_with_headers(
        &p,
        &entity,
        name,
        &extra,
        vec![Part::raw("listing", "text/x-asm", "listing.txt", listing.to_vec())],
    )
    .map_err(|e| format!("rewrite {name}: {e}"))
}

/// Семантический поиск по имени функции и телу.
pub fn cluster(dir: &Path, pattern: &str) -> Result<Vec<String>, String> {
    let pat = pattern.to_ascii_lowercase();
    let mut out = Vec::new();
    let rd = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    for e in rd.flatten() {
        let p = e.path();
        if !p.extension().map(|x| x == "eml").unwrap_or(false) {
            continue;
        }
        let b = EmlBox::open(&p)?;
        let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let hay = format!(
            "{} {}",
            name,
            b.section("listing").map(|s| String::from_utf8_lossy(s).to_string()).unwrap_or_default()
        )
        .to_ascii_lowercase();
        if hay.contains(&pat) {
            out.push(name);
        }
    }
    out.sort();
    Ok(out)
}

/// Все функции с их To-графом (для визуализации).
pub fn graph(dir: &Path) -> Result<Vec<(String, Vec<String>)>, String> {
    let mut out = Vec::new();
    let rd = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    for e in rd.flatten() {
        let p = e.path();
        if !p.extension().map(|x| x == "eml").unwrap_or(false) {
            continue;
        }
        let b = EmlBox::open(&p)?;
        let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let callees: Vec<String> = b
            .headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("X-Callees"))
            .flat_map(|(_, v)| v.split(',').map(|s| s.trim().to_string()))
            .collect();
        out.push((name, callees));
    }
    out.sort();
    Ok(out)
}

/// Дайджест типов по всему графу: func -> [X-Type-*]
pub fn type_map(dir: &Path) -> Result<Vec<(String, Vec<String>)>, String> {
    let mut out = Vec::new();
    let rd = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    for e in rd.flatten() {
        let p = e.path();
        if !p.extension().map(|x| x == "eml").unwrap_or(false) {
            continue;
        }
        let b = EmlBox::open(&p)?;
        let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let types: Vec<String> = b
            .headers
            .iter()
            .filter(|(k, _)| k.starts_with("X-Type-"))
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        if !types.is_empty() {
            out.push((name, types));
        }
    }
    out.sort();
    Ok(out)
}

/// Хэш тела функции (для диффинга версий).
pub fn body_hash(dir: &Path, name: &str) -> Result<String, String> {
    let b = EmlBox::open(&func_file(dir, name))?;
    Ok(hash_bytes(b.section("listing").ok_or("no listing")?))
}
