//! pack/unpack: директория <-> один .eml-контейнер.
//!
//! pack: каждый файл = секция, id/name = относительный путь (структура
//! сохраняется), MIME по расширению. Итог — один переносимый .eml с
//! hash-chain вместо zip/tar.
//!
//! unpack: извлекает секции по относительным путям; защита от path traversal
//! (абсолютные пути и ".." отвергаются) — контейнеры могут приходить из сети.

use crate::reader::EmlBox;
use crate::writer::{build_file, Part};
use std::path::{Component, Path, PathBuf};

fn mime_for(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref() {
        Some("json") => "application/json",
        Some("html") | Some("htm") => "text/html",
        Some("py") => "text/x-python",
        Some("rs") => "text/x-rust",
        Some("c") | Some("h") => "text/x-c",
        Some("md") => "text/markdown",
        Some("txt") | Some("log") => "text/plain",
        Some("sh") => "text/x-shellscript",
        Some("toml") => "text/x-toml",
        Some("yml") | Some("yaml") => "application/yaml",
        Some("css") => "text/css",
        Some("eml") => "message/rfc822",
        _ => "application/octet-stream",
    }
}

fn collect(root: &Path, cur: &Path, parts: &mut Vec<Part>, total: &mut u64) -> Result<(), String> {
    let rd = std::fs::read_dir(cur).map_err(|e| e.to_string())?;
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect(root, &p, parts, total)?;
        } else {
            let rel = p
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .to_string();
            let data = std::fs::read(&p).map_err(|e| format!("read {}: {e}", p.display()))?;
            *total += data.len() as u64;
            parts.push(Part::raw(&rel, mime_for(&p), &rel, data));
        }
    }
    Ok(())
}

/// Pack a directory tree into one container. Returns (file count, total bytes).
pub fn pack(dir: &Path, out: &Path, entity: &str) -> Result<(usize, u64), String> {
    if !dir.is_dir() {
        return Err(format!("not a directory: {}", dir.display()));
    }
    let mut parts = Vec::new();
    let mut total = 0u64;
    collect(dir, dir, &mut parts, &mut total)?;
    let n = parts.len();
    build_file(out, entity, &format!("pack: {}", dir.display()), parts)?;
    Ok((n, total))
}

/// Unpack all sections to relative paths under `out`. Returns file count.
pub fn unpack(container: &Path, out: &Path) -> Result<usize, String> {
    let b = EmlBox::open(container)?;
    let mut n = 0usize;
    for s in &b.sections {
        let rel = PathBuf::from(&s.id);
        if rel.is_absolute()
            || rel.components().any(|c| matches!(c, Component::ParentDir | Component::RootDir))
        {
            return Err(format!("unsafe path in container: {}", s.id));
        }
        let dest = out.join(&rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        if let Some(data) = b.section(&s.id) {
            std::fs::write(&dest, data).map_err(|e| format!("write {}: {e}", dest.display()))?;
            n += 1;
        }
    }
    Ok(n)
}
