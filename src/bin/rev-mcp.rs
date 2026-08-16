//! rev-mcp: MCP-сервер клеточного реверса — LLM видит только соседей клетки.
//!
//! stdio transport, JSON-RPC (line-delimited). Тулы:
//!   get_function(dir, name)   — заголовки + тело клетки
//!   get_callers(dir, name)    — кто вызывает
//!   get_callees(dir, name)    — кого вызывает
//!   cluster(dir, pattern)     — семантический поиск
//!   wave(dir, func, arg, ty, ticks) — волна типов
//!   types(dir)                — карта типов
//!   diff(dir_a, dir_b)        — диффинг версий
//!   graph(dir)                — весь call-граф

use emlbox::rev;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

fn tool(name: &str, description: &str, props: serde_json::Map<String, Value>) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": props }
    })
}

fn text(s: String) -> Value {
    json!({ "content": [{ "type": "text", "text": s }] })
}

fn run_tool(name: &str, args: &Value) -> Result<Value, String> {
    let s = |k: &str| args.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let dir = s("dir");
    let d = std::path::PathBuf::from(&dir);
    match name {
        "get_function" => {
            let name = s("name");
            let p = d.join(format!("{name}.eml"));
            let b = emlbox::reader::EmlBox::open(&p)?;
            let mut out = String::new();
            for (k, v) in &b.headers {
                if k.starts_with("X-") || k == "From" || k == "Subject" {
                    out.push_str(&format!("{k}: {v}\n"));
                }
            }
            if let Some(body) = b.section("listing") {
                out.push_str("\n--- listing ---\n");
                out.push_str(&String::from_utf8_lossy(&body));
            }
            Ok(text(out))
        }
        "get_callers" => {
            let callers = rev::callers(&d, &s("name"))?;
            Ok(text(format!("callers of {}: {}", s("name"), callers.join(", "))))
        }
        "get_callees" => {
            let g = rev::graph(&d)?;
            let callees = g.iter().find(|(n, _)| n == &s("name")).map(|(_, c)| c.clone()).unwrap_or_default();
            Ok(text(format!("callees of {}: {}", s("name"), callees.join(", "))))
        }
        "cluster" => {
            let hits = rev::cluster(&d, &s("pattern"))?;
            Ok(text(format!("{} matches: {}", hits.len(), hits.join(", "))))
        }
        "wave" => {
            let ticks: usize = args.get("ticks").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
            let hit = rev::type_mark(&d, &s("func"), &s("arg"), &s("ty"), ticks)?;
            Ok(text(format!("wave reached {}: {}", hit.len(), hit.join(", "))))
        }
        "types" => {
            let tm = rev::type_map(&d)?;
            let mut out = String::new();
            for (n, ts) in &tm {
                out.push_str(&format!("{n}: {}\n", ts.join(", ")));
            }
            if out.is_empty() {
                out = "no types yet".into();
            }
            Ok(text(out))
        }
        "diff" => {
            let (changed, added, removed) = rev::diff(&d, &std::path::PathBuf::from(s("dir_b")))?;
            Ok(text(format!(
                "changed ({}): {}\nadded ({}): {}\nremoved ({}): {}",
                changed.len(), changed.join(", "),
                added.len(), added.join(", "),
                removed.len(), removed.join(", ")
            )))
        }
        "graph" => {
            let g = rev::graph(&d)?;
            let mut out = String::new();
            for (n, c) in &g {
                out.push_str(&format!("{n} -> {}\n", c.join(", ")));
            }
            Ok(text(out))
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

fn main() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = msg.get("id").cloned();
        let mut reply = json!({ "jsonrpc": "2.0" });
        match method {
            "initialize" => {
                reply["id"] = id.unwrap_or(Value::Null);
                reply["result"] = json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "emlbox-rev-mcp", "version": "0.1.0" }
                });
            }
            "ping" => {
                reply["id"] = id.unwrap_or(Value::Null);
                reply["result"] = json!({});
            }
            "notifications/initialized" | "notifications/cancelled" => continue,
            "tools/list" => {
                reply["id"] = id.unwrap_or(Value::Null);
                reply["result"] = json!({
                    "tools": [
                        tool("get_function", "Headers + assembly body of one function cell", json_map(&[("dir", "string"), ("name", "string")])),
                        tool("get_callers", "Who calls this function (References)", json_map(&[("dir", "string"), ("name", "string")])),
                        tool("get_callees", "What this function calls (X-Callees)", json_map(&[("dir", "string"), ("name", "string")])),
                        tool("cluster", "Semantic search by name+body", json_map(&[("dir", "string"), ("pattern", "string")])),
                        tool("wave", "Propagate type up the call graph (call-site aware)", json_map(&[("dir", "string"), ("func", "string"), ("arg", "string"), ("ty", "string"), ("ticks", "integer")])),
                        tool("types", "Type map across all cells", json_map(&[("dir", "string")])),
                        tool("diff", "Version diff between two cell dirs", json_map(&[("dir", "string"), ("dir_b", "string")])),
                        tool("graph", "Full call graph", json_map(&[("dir", "string")])),
                    ]
                });
            }
            "tools/call" => {
                reply["id"] = id.unwrap_or(Value::Null);
                let name = msg.pointer("/params/name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let args = msg.pointer("/params/arguments").cloned().unwrap_or(Value::Null);
                match run_tool(&name, &args) {
                    Ok(r) => reply["result"] = r,
                    Err(e) => reply["error"] = json!({ "code": -32000, "message": e }),
                }
            }
            _ => {
                reply["id"] = id.unwrap_or(Value::Null);
                reply["error"] = json!({ "code": -32601, "message": format!("method not found: {method}") });
            }
        }
        let _ = writeln!(stdout, "{}", serde_json::to_string(&reply).unwrap_or_default());
        let _ = stdout.flush();
    }
}

fn json_map(pairs: &[(&str, &str)]) -> serde_json::Map<String, Value> {
    let mut m = serde_json::Map::new();
    for (k, t) in pairs {
        m.insert(k.to_string(), json!({ "type": t }));
    }
    m
}
