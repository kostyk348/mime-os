//! EML-IPC: события — это .eml-сообщения в spool-директории (локальная шина).
//! Конверт тот же, что у файлов: From/To/X-Event + JSON-тело. Позже локальный
//! spool заменяется транспортом без изменения формата (SMTP-меш, QUIC).

use crate::format::{find_blank, header_get, parse_headers};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MSG_SUFFIX: &str = ".msg.eml";
pub const DONE_SUFFIX: &str = ".done";

pub struct Message {
    pub from: String,
    pub to: String,
    pub event: String,
    pub body: Value,
    pub path: PathBuf,
}

/// Write an event message to the bus.
pub fn send(bus: &Path, from: &str, to: &str, event: &str, body: &Value) -> Result<PathBuf, String> {
    std::fs::create_dir_all(bus).map_err(|e| e.to_string())?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let safe_ev = event.replace([' ', '/', ':'], "_");
    let name = format!("{nanos:020}_{}.{safe_ev}{MSG_SUFFIX}", std::process::id());
    let path = bus.join(name);
    let mut buf = format!(
        "From: <{from}@system.local>\r\nTo: <{to}>\r\nX-Event: {event}\r\n\
         X-EMLBox-Msg: v1\r\nContent-Type: application/json\r\n\r\n"
    )
    .into_bytes();
    buf.extend_from_slice(serde_json::to_vec(body).map_err(|e| e.to_string())?.as_slice());
    std::fs::write(&path, &buf).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Parse a bus message file.
pub fn parse(path: &Path) -> Result<Message, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let (end, sep) = find_blank(&data).ok_or("message: no blank line after headers")?;
    let headers = parse_headers(&data[..end]);
    let body = if end + sep < data.len() {
        serde_json::from_slice(&data[end + sep..])
            .unwrap_or_else(|_| Value::Object(serde_json::Map::new()))
    } else {
        Value::Object(serde_json::Map::new())
    };
    Ok(Message {
        from: header_get(&headers, "From")
            .unwrap_or("")
            .trim_matches(|c| c == '<' || c == '>' || c == ' ')
            .to_string(),
        to: header_get(&headers, "To")
            .unwrap_or("")
            .trim_matches(|c| c == '<' || c == '>' || c == ' ')
            .to_string(),
        event: header_get(&headers, "X-Event").unwrap_or("").to_string(),
        body,
        path: path.to_path_buf(),
    })
}

/// List pending (unprocessed) messages, sorted.
pub fn list(bus: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(bus) {
        for e in rd.flatten() {
            let p = e.path();
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if name.ends_with(MSG_SUFFIX) && !name.ends_with(DONE_SUFFIX) {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Consume a message (rename to .done).
pub fn mark_done(p: &Path) -> Result<(), String> {
    let done = PathBuf::from(format!("{}{}", p.display(), DONE_SUFFIX));
    std::fs::rename(p, &done).map_err(|e| e.to_string())
}
