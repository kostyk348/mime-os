//! SMTP-мост: контейнеры путешествуют как настоящие письма.
//!
//! mail pack   — экспортировать дельта-блоки писателя в валидное MIME-письмо
//!               (multipart/mixed, блоки дословно) — его можно отправить любым
//!               почтовым клиентом / SMTP-релеем.
//! mail apply  — применить блоки из письма в контейнер (идемпотентно).
//! mail receive— просканировать Maildir (Thunderbird/offlineimap) и применить.
//!
//! Никаких TLS-зависимостей: отправка наружу — через существующий MUA/SMTP,
//! приём — из локального Maildir. Шифрование при желании — X-Encoding блоков.

use crate::format::{find_blank, header_get, parse_headers};
use crate::reader::EmlBox;
use crate::writer::append_block_w;
use std::path::Path;

const BOUNDARY: &str = "EMLBOX_SYNC_v1";

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Собрать письмо с дельтами писателя (seq > since). Returns raw MIME bytes.
pub fn pack(container: &Path, writer: &str, to: &str, since: u64) -> Result<Vec<u8>, String> {
    let b = EmlBox::open(container)?;
    let entity = b.entity().ok_or("no entity")?;
    let blocks = crate::sync::export(container, writer, since)?;
    let mut out = format!(
        "From: <{entity}>\r\nTo: <{to}>\r\nSubject: [EMLBox-Sync] {entity}\r\n\
         X-EMLBox-Sync: v1\r\nX-Writer-ID: {writer}\r\nDate: {}\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"{BOUNDARY}\"\r\n\r\n",
        now_ts()
    )
    .into_bytes();
    for (seq, block) in &blocks {
        out.extend_from_slice(format!("--{BOUNDARY}\r\nContent-Type: application/x-emlbox-delta\r\nContent-ID: <block-{seq}>\r\n\r\n").as_bytes());
        out.extend_from_slice(block);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    Ok(out)
}

/// Байтовый split по разделителю (для бинарных частей).
fn split_bytes<'a>(data: &'a [u8], delim: &[u8]) -> Vec<&'a [u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i + delim.len() <= data.len() {
        if &data[i..i + delim.len()] == delim {
            out.push(&data[start..i]);
            i += delim.len();
            start = i;
        } else {
            i += 1;
        }
    }
    out.push(&data[start..]);
    out
}

/// Извлечь дельта-блоки из MIME-письма: Vec<(writer, seq, block)>.
pub fn unpack(bytes: &[u8]) -> Result<Vec<(String, u64, Vec<u8>)>, String> {
    let (end, sep) = find_blank(bytes).ok_or("mail: no envelope headers")?;
    let h = parse_headers(&bytes[..end]);
    if header_get(&h, "X-EMLBox-Sync").is_none() {
        return Ok(Vec::new());
    }
    let boundary = header_get(&h, "Content-Type")
        .and_then(|ct| ct.split("boundary=").nth(1))
        .map(|b| b.trim_matches('"').to_string())
        .unwrap_or_else(|| BOUNDARY.to_string());
    let body = &bytes[end + sep..];
    let mut blocks = Vec::new();
    let delim = format!("--{boundary}");
    for part in split_bytes(body, delim.as_bytes()) {
        let mut p = part;
        if p.starts_with(b"--") {
            p = &p[2..];
        }
        while p.starts_with(b"\r") || p.starts_with(b"\n") {
            p = &p[1..];
        }
        if p.trim_ascii().is_empty() || p == b"--" || p.starts_with(b"--\r\n") {
            continue;
        }
        let (pend, psep) = match find_blank(p) {
            Some(x) => x,
            None => continue,
        };
        let ph = parse_headers(&p[..pend]);
        if !header_get(&ph, "Content-Type").map(|c| c.starts_with("application/x-emlbox-delta")).unwrap_or(false) {
            continue;
        }
        let block = &p[pend + psep..];
        let block = trim_crlf(block);
        if block.is_empty() {
            continue;
        }
        let writer = crate::format::block_header(block, "X-Writer-ID")
            .unwrap_or_else(|| crate::format::DEFAULT_WRITER.to_string());
        let seq: u64 = crate::format::block_header(block, "X-Delta-Seq")
            .ok_or("mail block: no seq")?
            .trim()
            .parse()
            .map_err(|_| "mail block: bad seq")?;
        blocks.push((writer, seq, block.to_vec()));
    }
    Ok(blocks)
}

fn trim_crlf(mut b: &[u8]) -> &[u8] {
    while b.ends_with(b"\n") || b.ends_with(b"\r") {
        b = &b[..b.len() - 1];
    }
    b
}

/// Применить блоки письма в контейнер. Returns (applied, pending).
pub fn apply(container: &Path, bytes: &[u8]) -> Result<(usize, usize), String> {
    let blocks = unpack(bytes)?;
    let mut applied = 0;
    let mut pending = 0;
    let b = EmlBox::open(container)?;
    for (writer, seq, block) in &blocks {
        // идемпотентность: уже есть writer#seq
        let exists = b.tail_entries().iter().any(|e| &e.writer == writer && e.seq == *seq);
        if exists {
            applied += 1;
            continue;
        }
        match append_block_w(container, writer, block) {
            Ok(_) => applied += 1,
            Err(_) => pending += 1, // вне порядка — ждёт предшественника в следующем письме
        }
    }
    Ok((applied, pending))
}

/// Сканировать Maildir (new/ + cur/) на письма с X-EMLBox-Sync и применить.
/// Обработанные перемещаются в <maildir>/processed/. Returns (applied, pending).
pub fn receive(container: &Path, maildir: &Path) -> Result<(usize, usize), String> {
    let mut applied_total = 0;
    let mut pending_total = 0;
    let proc_dir = maildir.join("processed");
    std::fs::create_dir_all(&proc_dir).map_err(|e| e.to_string())?;
    for sub in ["new", "cur"] {
        let dir = maildir.join(sub);
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            let bytes = match std::fs::read(&p) {
                Ok(d) => d,
                Err(_) => continue,
            };
            match apply(container, &bytes) {
                Ok((a, pnd)) => {
                    if a > 0 || pnd > 0 {
                        applied_total += a;
                        pending_total += pnd;
                        let _ = std::fs::rename(&p, proc_dir.join(format!("{}-{}", now_ts(), p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())));
                    }
                }
                Err(_) => continue,
            }
        }
    }
    Ok((applied_total, pending_total))
}
