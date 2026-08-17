//! Сайт-генератор: посты = .eml-контейнеры -> статический сайт.
//!
//! Пост: контейнер с Subject=заголовок, X-Tag: a,b, X-Timestamp: unix,
//! секция body (markdown). site собирает index.html, post/<n>.html, tags.html.
//! Мини-markdown без зависимостей.

use crate::reader::EmlBox;
use crate::writer::{build_file_with_headers, Part};
use std::path::Path;

pub struct Post {
    pub name: String,
    pub title: String,
    pub ts: u64,
    pub tags: Vec<String>,
    pub body: String,
}

/// Создать контейнер-пост.
pub fn new_post(path: &Path, title: &str, tags: &[String], body: &str) -> Result<(), String> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let extra = format!("X-Tag: {}\r\nX-Timestamp: {ts}\r\n", tags.join(", "));
    let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    build_file_with_headers(
        path,
        &name,
        title,
        &extra,
        vec![Part::raw("body", "text/markdown", "body.md", body.as_bytes().to_vec())],
    )
}

/// Прочитать все посты из каталога.
pub fn read_posts(dir: &Path) -> Result<Vec<Post>, String> {
    let mut out = Vec::new();
    let rd = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().map(|x| x == "eml").unwrap_or(false) {
            if let Ok(b) = EmlBox::open(&p) {
                let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                let title = b.header("Subject").unwrap_or(&name).to_string();
                let ts: u64 = b
                    .header("X-Timestamp")
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0);
                let tags: Vec<String> = b
                    .header("X-Tag")
                    .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
                    .unwrap_or_default();
                let body = b
                    .section("body")
                    .map(|s| String::from_utf8_lossy(&s).to_string())
                    .unwrap_or_default();
                out.push(Post { name, title, ts, tags, body });
            }
        }
    }
    out.sort_by_key(|p| std::cmp::Reverse(p.ts));
    Ok(out)
}

/// Собрать сайт. Returns число постов.
pub fn build(posts_dir: &Path, out_dir: &Path) -> Result<usize, String> {
    let posts = read_posts(posts_dir)?;
    std::fs::create_dir_all(out_dir.join("post")).map_err(|e| e.to_string())?;
    let page = |title: &str, body: &str| -> String {
        format!(
            "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>{title}</title>\
             <style>body{{max-width:720px;margin:2rem auto;padding:0 1rem;font-family:system-ui,sans-serif;line-height:1.6}}\
             code{{background:#f0f0f0;padding:2px 4px}}pre{{background:#f4f4f4;padding:1rem;overflow-x:auto}}</style></head>\
             <body>{body}</body></html>"
        )
    };
    // index.html
    let mut items = String::new();
    let mut tag_map: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    for p in &posts {
        for t in &p.tags {
            tag_map.entry(t.clone()).or_default().push(p.name.clone());
        }
        let date = if p.ts > 0 { epoch_date(p.ts) } else { "?".into() };
        let tags = p
            .tags
            .iter()
            .map(|t| format!("<a href=\"../tags.html\">#{}</a>", esc(t)))
            .collect::<Vec<_>>()
            .join(" ");
        items.push_str(&format!(
            "<li><a href=\"post/{}.html\">{}</a> <small>({})</small> {tags}</li>",
            p.name,
            esc(&p.title),
            date,
        ));
    }
    let index = page("Index", &format!("<h1>Blog</h1><ul>{items}</ul>"));
    std::fs::write(out_dir.join("index.html"), &index).map_err(|e| e.to_string())?;
    // tags.html
    let mut tag_html = String::new();
    for (tag, names) in &tag_map {
        tag_html.push_str(&format!("<h2 id=\"{}\">{}</h2><ul>", esc(tag), esc(tag)));
        for n in names {
            let p = posts.iter().find(|p| &p.name == n).unwrap();
            tag_html.push_str(&format!("<li><a href=\"post/{}.html\">{}</a></li>", esc(n), esc(&p.title)));
        }
        tag_html.push_str("</ul>");
    }
    std::fs::write(out_dir.join("tags.html"), &page("Tags", &tag_html)).map_err(|e| e.to_string())?;
    // посты
    for p in &posts {
        let html = md(&p.body);
        let tags = p.tags.iter().map(|t| format!("<a href=\"../tags.html\">#{}</a>", esc(t))).collect::<Vec<_>>().join(" ");
        let body = format!("<h1>{}</h1><p><small>{} {}</small></p>{}{}", esc(&p.title), epoch_date(p.ts), tags, html, "<p><a href=\"../index.html\">← index</a></p>");
        std::fs::write(out_dir.join("post").join(format!("{}.html", p.name)), &page(&p.title, &body)).map_err(|e| e.to_string())?;
    }
    Ok(posts.len())
}

/// Экспорт постов в Hugo content: content/posts/<name>.md с YAML front matter
/// (title, date, tags) — Hugo/PaperMod соберёт сам. Returns число постов.
pub fn export_hugo(posts_dir: &Path, content_dir: &Path) -> Result<usize, String> {
    let posts = read_posts(posts_dir)?;
    let out = content_dir.join("posts");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    for p in &posts {
        let date = if p.ts > 0 {
            let (y, m, d) = civil_date(p.ts);
            format!("{y:04}-{m:02}-{d:02}")
        } else {
            "1970-01-01".to_string()
        };
        let tags = p.tags.iter().map(|t| format!("    - \"{}\"", t.replace('"', "\\\""))).collect::<Vec<_>>().join("\n");
        let mut md = format!(
            "---\ntitle: \"{}\"\ndate: {date}\ntags:\n{tags}\n---\n\n{}",
            p.title.replace('"', "\\\""),
            p.body
        );
        if !md.ends_with('\n') {
            md.push('\n');
        }
        let f = out.join(format!("{}.md", p.name));
        std::fs::write(&f, md).map_err(|e| e.to_string())?;
    }
    Ok(posts.len())
}

fn civil_date(ts: u64) -> (i64, i64, i64) {
    let days = (ts / 86400) as i64;
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn epoch_date(ts: u64) -> String {
    let (y, m, d) = civil_date(ts);
    let h = (ts % 86400) / 3600;
    let min = (ts % 3600) / 60;
    format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02} UTC")
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Мини-markdown: заголовки, код, списки, параграфы, ссылки, жирный.
pub fn md(src: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    let mut in_list = false;
    let close_list = |out: &mut String, in_list: &mut bool| {
        if *in_list {
            out.push_str("</ul>");
            *in_list = false;
        }
    };
    for line in src.lines() {
        let line = line.trim_end();
        if line.starts_with("```") {
            close_list(&mut out, &mut in_list);
            if in_code {
                out.push_str("</code></pre>");
                in_code = false;
            } else {
                out.push_str("<pre><code>");
                in_code = true;
            }
            continue;
        }
        if in_code {
            out.push_str(&esc(line));
            out.push('\n');
            continue;
        }
        if let Some(rest) = line.strip_prefix("### ") {
            close_list(&mut out, &mut in_list);
            out.push_str(&format!("<h3>{}</h3>", inline(rest)));
        } else if let Some(rest) = line.strip_prefix("## ") {
            close_list(&mut out, &mut in_list);
            out.push_str(&format!("<h2>{}</h2>", inline(rest)));
        } else if let Some(rest) = line.strip_prefix("# ") {
            close_list(&mut out, &mut in_list);
            out.push_str(&format!("<h1>{}</h1>", inline(rest)));
        } else if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            if !in_list {
                out.push_str("<ul>");
                in_list = true;
            }
            out.push_str(&format!("<li>{}</li>", inline(rest)));
        } else if line.trim().is_empty() {
            close_list(&mut out, &mut in_list);
        } else {
            close_list(&mut out, &mut in_list);
            out.push_str(&format!("<p>{}</p>", inline(line)));
        }
    }
    close_list(&mut out, &mut in_list);
    if in_code {
        out.push_str("</code></pre>");
    }
    out
}

fn inline(s: &str) -> String {
    // [text](url) → ссылка
    let mut out = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        out.push_str(&esc(&rest[..open]));
        rest = &rest[open + 1..];
        let (label, after) = match rest.split_once(']') {
            Some(x) => x,
            None => {
                out.push('[');
                break;
            }
        };
        if let Some(url) = after.strip_prefix("(").and_then(|a| a.split_once(')')) {
            out.push_str(&format!("<a href=\"{}\">{}</a>", esc(url.0), esc(label)));
            rest = url.1;
        } else {
            out.push_str(&esc(label));
            rest = after;
        }
    }
    out.push_str(&esc(rest));
    // **bold**
    let parts: Vec<&str> = s.split("**").collect();
    if parts.len() >= 3 {
        let mut rebuilt = String::new();
        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                rebuilt.push_str(if i % 2 == 1 { "<b>" } else { "</b>" });
            }
            rebuilt.push_str(&esc(part));
        }
        out = rebuilt;
    }
    out
}
