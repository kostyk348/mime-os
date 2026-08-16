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
    /// call-site аргументы: для каждого вызова (callee, arg_idx -> src).
    /// src — регистр/значение, передаваемое в аргументный регистр callee.
    pub call_sites: Vec<(String, Vec<(usize, String)>)>,
}

/// Регистр-параметр вызывающей функции по SysV x86-64 (arg0..arg5).
pub fn param_reg(arg: usize) -> Option<&'static str> {
    match arg {
        0 => Some("rdi"),
        1 => Some("rsi"),
        2 => Some("rdx"),
        3 => Some("rcx"),
        4 => Some("r8"),
        5 => Some("r9"),
        _ => None,
    }
}

fn arg_regs() -> [&'static str; 6] {
    ["rdi", "rsi", "rdx", "rcx", "r8", "r9"]
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
                cur = Some(RevFunc { name, callees: Vec::new(), body: Vec::new(), call_sites: Vec::new() });
                continue;
            }
        }
        if let Some(f) = cur.as_mut() {
            // извлечь имена из call-инструкций: "call 401090 <net_send@plt>"
            if let Some(ci) = trimmed.find("call") {
                let rest = &trimmed[ci + 4..];
                if let Some(l) = rest.find('<') {
                    if let Some(r) = rest[l + 1..].find('>') {
                        let cname = rest[l + 1..l + 1 + r].to_string();
                        if !f.callees.contains(&cname) {
                            f.callees.push(cname.clone());
                        }
                        // call-site аргументы: mov <argreg>, <src> до call
                        let site = scan_mov_args(&f.body);
                        f.call_sites.push((cname, site));
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

/// Собрать (arg_idx -> src) из mov-инструкций перед вызовом: последние строки
/// тела, где "mov rdi, X" / "mov edi, X". Если аргументных mov'ов нет —
/// дефолтная передача (argN -> param_reg(N), передаются как есть).
fn scan_mov_args(body: &[String]) -> Vec<(usize, String)> {
    let regs = arg_regs();
    let mut out: Vec<(usize, String)> = Vec::new();
    for line in body.iter().rev().take(10) {
        let l = line.to_ascii_lowercase();
        let (_, after) = match l.split_once("mov") {
            Some(x) => x,
            None => continue,
        };
        let parts: Vec<&str> = after.split(',').map(|s| s.trim()).collect();
        if parts.len() != 2 {
            continue;
        }
        let dst = parts[0].split_whitespace().last().unwrap_or("");
        let src = parts[1].split_whitespace().next().unwrap_or("");
        let arg = regs.iter().position(|r| *r == dst || format!("e{}", &r[1..]) == dst);
        if let Some(a) = arg {
            if !out.iter().any(|(i, _)| *i == a) {
                out.push((a, src.to_string()));
            }
        }
        if out.len() >= 4 {
            break;
        }
    }
    if out.is_empty() {
        // нет mov — аргументы передаются как есть (текущие параметры)
        for (i, r) in regs.iter().enumerate() {
            out.push((i, r.to_string()));
        }
    }
    out
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

fn site_to_header(sites: &[(String, Vec<(usize, String)>)]) -> String {
    let mut out = String::new();
    for (callee, site) in sites {
        let parts: Vec<String> = site.iter().map(|(i, s)| format!("{i}={s}")).collect();
        out.push_str(&format!("X-Call-Site: {callee}:{}\r\n", parts.join(",")));
    }
    out
}

/// Читать X-Call-Site заголовки клетки: callee -> [(arg, src)].
fn site_map(dir: &Path, name: &str) -> Result<Vec<(String, Vec<(usize, String)>)>, String> {
    let p = func_file(dir, name);
    if !p.exists() {
        return Ok(Vec::new());
    }
    let b = EmlBox::open(&p)?;
    let mut out = Vec::new();
    for (k, v) in &b.headers {
        if k.eq_ignore_ascii_case("X-Call-Site") {
            let (callee, rest) = match v.split_once(':') {
                Some(x) => x,
                None => continue,
            };
            let site = rest
                .split(',')
                .filter_map(|kv| {
                    let (i, s) = kv.split_once('=')?;
                    Some((i.trim().parse().ok()?, s.trim().to_string()))
                })
                .collect();
            out.push((callee.trim().to_string(), site));
        }
    }
    Ok(out)
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
        extra.push_str(&site_to_header(&f.call_sites));
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
/// Передаёт ли функция `f` свой параметр в аргумент `arg_n` вызова `callee`?
/// Мини-dataflow: src регистра resolve-ится назад по mov/lea цепочкам к
/// параметру ([rbp+0x10..]), локали ([rbp-..]) или константе.
fn passing_param(dir: &Path, f: &str, callee: &str, arg_n: usize) -> Option<usize> {
    let sites = site_map(dir, f).ok()?;
    for (cname, site) in sites {
        if cname != callee {
            continue;
        }
        for (a, src) in site {
            if a != arg_n {
                continue;
            }
            // передача "как есть": src == регистр-параметр той же позиции
            let src_clean = src.to_ascii_lowercase();
            let src_clean = src_clean.trim_matches(|c: char| !c.is_ascii_alphanumeric()).to_string();
            if Some(src_clean.as_str()) == param_reg(arg_n) {
                return Some(arg_n);
            }
            // иначе: мини-dataflow по mov/lea цепочкам
            let body = body_of(dir, f)?;
            let slots = slot_map(&body);
            if let Some(i) = resolve_param(&body, &src, &slots) {
                return Some(i);
            }
        }
    }
    None
}

/// Тело функции f (строки листинга).
fn body_of(dir: &Path, f: &str) -> Option<Vec<String>> {
    let b = EmlBox::open(&func_file(dir, f)).ok()?;
    let s = b.section("listing")?;
    Some(String::from_utf8_lossy(s).lines().map(|l| l.trim().to_string()).collect())
}

/// Пролог -O0: "mov QWORD PTR [rbp-0x8],rdi" — спасённый параметр.
/// Слот "-0x8" → arg0 (rdi). Возвращает map слот-ключ -> позиция параметра.
fn slot_map(body: &[String]) -> std::collections::HashMap<String, usize> {
    let regs = arg_regs();
    let mut map = std::collections::HashMap::new();
    for line in body.iter().take(8) {
        let l = line.to_ascii_lowercase();
        let (_, after) = match l.split_once("mov") {
            Some(x) => x,
            None => continue,
        };
        let parts: Vec<&str> = after.split(',').map(|s| s.trim()).collect();
        if parts.len() != 2 {
            continue;
        }
        let dst = parts[0].split_whitespace().last().unwrap_or("");
        let key = if let Some(off) = rbp_slot(dst) {
            off
        } else {
            continue;
        };
        let src = parts[1].split_whitespace().next().unwrap_or("");
        if let Some(i) = regs.iter().position(|r| *r == src) {
            map.entry(key).or_insert(i);
        }
    }
    map
}

/// Смещение [rbp-0x8] → "-0x8", [rbp+0x10] → "+0x10".
fn rbp_slot(s: &str) -> Option<String> {
    let inner = s.trim().strip_prefix('[')?.strip_suffix(']')?;
    let (base, off) = inner.split_once(['+', '-'])?;
    if base.trim() != "rbp" {
        return None;
    }
    Some(format!("{}{}", if inner.contains('-') { "-" } else { "+" }, off.trim().trim_start_matches("0x")))
}

/// Resolve регистра к параметру вызывающей функции (i) через цепочки
/// mov/lea назад по телу. [rbp-0x8] — слот пролога (спасённый параметр);
/// [rbp+0x10] — аргумент SysV; [rbp-..] без слота — локаль; константа — None.
fn resolve_param(body: &[String], src: &str, slots: &std::collections::HashMap<String, usize>) -> Option<usize> {
    let mut reg = src.to_ascii_lowercase();
    reg = reg.trim_matches(|c: char| !c.is_ascii_alphanumeric()).to_string();
    let mut calls = 0usize;
    for line in body.iter().rev().take(12) {
        let l = line.to_ascii_lowercase();
        if l.contains("call") && !l.contains("mov") && !l.contains("lea") {
            calls += 1;
            // первый call — тот, к которому идём; второй — def потерян
            if calls > 1 {
                return None;
            }
            continue;
        }
        let idx = if l.contains("mov") {
            l.find("mov")?
        } else if l.contains("lea") {
            l.find("lea")?
        } else {
            continue;
        };
        let after = &l[idx + 3..];
        if !after.contains(&reg) {
            continue;
        }
        let parts: Vec<&str> = after.split(',').map(|s| s.trim()).collect();
        if parts.len() != 2 {
            return None;
        }
        let dst = parts[0].split_whitespace().last()?.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        if dst != reg {
            continue;
        }
        let s = parts[1].trim();
        // "QWORD PTR [rbp-0x8]" → "[-0x8]"; "[rbp-0x20]" уже в скобках
        if let Some(bi) = s.rfind('[') {
            let bstr = &s[bi..];
            // [rbp-0x8] — слот пролога → параметр; [rbp+0x10] — аргумент SysV
            if let Some(key) = rbp_slot(bstr) {
                if key.starts_with('-') {
                    return slots.get(&key).copied();
                }
                if let Some(off) = stack_param_offset(bstr) {
                    return Some(off);
                }
            }
            return None; // локаль без слота
        }
        // константа
        if s.starts_with("0x") || s.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            return None;
        }
        // регистр: движемся дальше
        reg = s.trim_matches(|c: char| !c.is_ascii_alphanumeric()).to_string();
        continue;
    }
    None
}

/// [rbp+0x10] → 0, [rbp+0x18] → 1, ... (аргументы SysV после push rbp)
fn stack_param_offset(s: &str) -> Option<usize> {
    let s = s.trim();
    let inner = if let Some(i) = s.strip_prefix('[') {
        i.strip_suffix(']')?
    } else {
        return None;
    };
    let (base, off) = inner.split_once('+')?;
    if base.trim() != "rbp" {
        return None;
    }
    let n = i64::from_str_radix(off.trim().trim_start_matches("0x"), 16).ok()?;
    if n < 0x10 {
        return None;
    }
    Some(((n - 0x10) / 8) as usize)
}

/// Волновое распространение типов: пометить `func` типом `ty` для аргумента
/// `arg` (например "arg0"), затем BFS по References (вызывающие) на `max_depth`
/// тактов. Точная волна: вызывающая функция типизируется только если реально
/// передаёт свой параметр в помеченный аргумент (call-site анализ).
/// depth=0 — только сама функция. Возвращает "func (argN)" — достигнутые.
pub fn type_mark(dir: &Path, func: &str, arg: &str, ty: &str, max_depth: usize) -> Result<Vec<String>, String> {
    let arg_idx: usize = arg.trim_start_matches("arg").parse().unwrap_or(0);
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, usize, usize)> = VecDeque::new(); // (name, marked_arg, depth)
    let mut reached = Vec::new();
    queue.push_back((func.to_string(), arg_idx, 0));
    seen.insert(func.to_string());
    while let Some((name, marked_arg, depth)) = queue.pop_front() {
        if depth > max_depth {
            continue;
        }
        let p = func_file(dir, &name);
        if !p.exists() {
            continue;
        }
        let b = EmlBox::open(&p)?;
        let mut types: Vec<(String, String)> = Vec::new();
        for (k, v) in &b.headers {
            let k = k.to_string();
            if k.starts_with("X-Type-") {
                types.push((k, v.clone()));
            }
        }
        let tkey = format!("X-Type-arg{marked_arg}");
        if !types.iter().any(|(k, _)| k == &tkey) {
            types.push((tkey.clone(), ty.to_string()));
        }
        reached.push(format!("{name} (arg{marked_arg})"));
        rewrite(dir, &name, &types)?;
        // распространить вверх: кто вызывает name и реально передаёт параметр
        let callers = callers(dir, &name)?;
        for c in callers {
            if let Some(i) = passing_param(dir, &c, &name, marked_arg) {
                if seen.insert(c.clone()) {
                    queue.push_back((c, i, depth + 1));
                }
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

/// Нормализованное тело: без адресной колонки, hex-байтов и адресов в
/// операндах ("call 1119 <net_send>" → "call <net_send>") — стабильно при
/// пересборке без изменений кода.
pub fn normalize_body(body: &str) -> String {
    let mut out = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // последняя колонка objdump (мнемоника + операнды)
        let m = match line.rsplit('\t').next() {
            Some(m) => m.trim(),
            None => continue,
        };
        // убрать комментарии objdump: " ... # <_IO_stdin_used+0x4>"
        let m = match m.find(" #") {
            Some(i) => &m[..i],
            None => m,
        };
        // убрать адреса-цели: "call 1119 <net_send>" → "call <net_send>"
        let m = replace_addr_targets(m);
        // убрать RIP-относительные смещения: "[rip+0xd18]" → "[rip+ADDR]"
        let m = replace_rip(&m);
        out.push(m.to_string());
    }
    out.join("\n")
}

fn replace_addr_targets(m: &str) -> String {
    let mut s = m.to_string();
    loop {
        let bytes = s.as_bytes();
        let mut done = false;
        for i in 0..bytes.len().saturating_sub(1) {
            if bytes[i] == b'<' && i > 0 && bytes[i - 1].is_ascii_whitespace() {
                // найти начало hex-токена перед пробелом
                let mut j = i - 1;
                while j > 0 && bytes[j - 1].is_ascii_whitespace() {
                    j -= 1;
                }
                // j указывает на пробел; токен перед ним
                let mut t = j - 1;
                while t > 0 && !bytes[t - 1].is_ascii_whitespace() {
                    t -= 1;
                }
                if t < j && is_hex(&s[t..j]) {
                    s.replace_range(t..j, "");
                    done = true;
                    break;
                }
            }
        }
        if !done {
            break;
        }
    }
    s
}

fn is_hex(t: &str) -> bool {
    !t.is_empty() && t.chars().all(|c| c.is_ascii_hexdigit())
}

/// "[rip+0xd18]" / "[rip-0x10]" → "[rip+ADDR]" — позиционно-зависимые смещения.
fn replace_rip(m: &str) -> String {
    let mut s = m.to_string();
    loop {
        let bytes = s.as_bytes();
        let mut done = false;
        for i in 0..bytes.len() {
            if bytes[i] == b'[' {
                if let Some(e) = s[i + 1..].find(']') {
                    let inner = &s[i + 1..i + 1 + e];
                    if inner.starts_with("rip") && (inner.contains('+') || inner.contains('-')) {
                        let (base, off) = match inner.split_once(['+', '-']) {
                            Some(x) => x,
                            None => break,
                        };
                        if base == "rip" && is_hex(off.trim().trim_start_matches("0x")) {
                            s.replace_range(i + 1..i + 1 + e, "rip+ADDR");
                            done = true;
                            break;
                        }
                    }
                }
            }
        }
        if !done {
            break;
        }
    }
    s
}

/// Хэш нормализованного тела функции (для диффинга версий).
pub fn body_hash(dir: &Path, name: &str) -> Result<String, String> {
    let b = EmlBox::open(&func_file(dir, name))?;
    let body = b.section("listing").ok_or("no listing")?;
    Ok(hash_bytes(normalize_body(&String::from_utf8_lossy(body)).as_bytes()))
}

/// Диффинг версий: какие функции изменились / добавились / удалились.
pub fn diff(dir_a: &Path, dir_b: &Path) -> Result<(Vec<String>, Vec<String>, Vec<String>), String> {
    let names = |dir: &Path| -> Result<Vec<String>, String> {
        let rd = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "eml").unwrap_or(false) {
                out.push(p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
            }
        }
        out.sort();
        Ok(out)
    };
    let a = names(dir_a)?;
    let b = names(dir_b)?;
    let mut changed = Vec::new();
    let mut added = Vec::new();
    let mut removed = Vec::new();
    for n in &a {
        if !b.contains(n) {
            removed.push(n.clone());
        }
    }
    for n in &b {
        if !a.contains(n) {
            added.push(n.clone());
        }
    }
    for n in &a {
        if b.contains(n) && body_hash(dir_a, n)? != body_hash(dir_b, n)? {
            changed.push(n.clone());
        }
    }
    Ok((changed, added, removed))
}

#[cfg(test)]
mod tests {
    use super::*;

    // objdump-стиль с -O0 паттерном: параметры спасены в слоты пролога
    const O0_LISTING: &str = r#"
0000000000001119 <net_send>:
    1119:	push   rbp
    111a:	ret

000000000000111b <player_take_damage>:
    111b:	push   rbp
    111c:	mov    QWORD PTR [rbp-0x8],rdi
    1120:	mov    rax,QWORD PTR [rbp-0x8]
    1124:	mov    rsi,rax
    1127:	mov    edi,0x21
    1129:	call   1119 <net_send>
    112e:	ret

000000000000112f <main>:
    112f:	push   rbp
    1130:	lea    rax,[rbp-0x20]
    1134:	mov    rdi,rax
    1137:	call   111b <player_take_damage>
    113c:	ret
"#;

    #[test]
    fn wave_respects_call_site_dataflow() {
        let dir = std::env::temp_dir().join(format!("rev_df_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        build(&dir, O0_LISTING).unwrap();

        // net_send.arg1 (data) ← player_take_damage передаёт свой arg0 (slot -8)
        let hit = type_mark(&dir, "net_send", "arg1", "void*", 5).unwrap();
        assert_eq!(hit, vec!["net_send (arg1)".to_string(), "player_take_damage (arg0)".to_string()]);
        // main передаёт ЛОКАЛЬ ([rbp-0x20]), не параметр → НЕ типизируется
        let tm = type_map(&dir).unwrap();
        assert_eq!(tm.len(), 2);
        for (name, types) in &tm {
            assert!(types.contains(&"X-Type-arg1=void*".to_string()) || types.contains(&"X-Type-arg0=void*".to_string()), "{name}: {types:?}");
        }
    }
}

#[cfg(test)]
mod norm_tests {
    use super::*;
    #[test]
    fn normalize_rips_and_targets() {
        let a = normalize_body("1058:\t48 8d 3d 34 02 00 00 \tlea    rdi,[rip+0x234]        # 1293 <main>");
        let b = normalize_body("1058:\t48 8d 3d 44 02 00 00 \tlea    rdi,[rip+0x244]        # 12a3 <main>");
        eprintln!("A: [{a}]");
        eprintln!("B: [{b}]");
        assert_eq!(a, b);
    }
}
