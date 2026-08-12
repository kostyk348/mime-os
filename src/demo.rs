//! Demo: one retro arcade app as a SINGLE .eml — GUI + logic + KV + raw binary,
//! plus a delta tail showing in-self persistence.

use crate::kv;
use crate::reader::EmlBox;
use crate::writer::{build_file, build_file_with_headers, Part};
use serde_json::json;
use std::path::Path;

const VIEW_HTML: &str = r#"<!DOCTYPE html>
<html>
<head><title>RETRO ARCADE</title></head>
<body style="background:#000;color:#0f0;font-family:monospace">
  <h1>MIME-OS / EMLBox demo</h1>
  <canvas id="viewport" width="320" height="240"></canvas>
  <br/>
  <button emlp-event="CLICK" emlp-target="<logic>">START</button>
  <button emlp-event="CLICK" emlp-target="<logic>">FIRE</button>
  <div id="hud">hp: <span data-bind="player_1.hp"></span></div>
</body>
</html>"#;

const LOGIC_PY: &str = r#"# logic.eml — event-driven logic, HAL-agnostic
def on_event(hal, state, event):
    kind = event.get("event")
    if kind == "MOVE":
        return [
            {"op": "set", "table": "state", "key": "x",
             "value": state.get("x", 0) + event.get("dx", 0)},
            {"op": "set", "table": "state", "key": "y",
             "value": state.get("y", 0) + event.get("dy", 0)},
        ]
    if kind == "FIRE":
        shots = state.get("shots", 0) + 1
        hal["emit"]({"to": "<view>", "event": "SHOT",
                     "data": {"x": state.get("x", 0), "y": state.get("y", 0), "shots": shots}})
        return [{"op": "set", "table": "state", "key": "shots", "value": shots}]
    return []
"#;

const STATE_JSON: &str = r#"{"x": 42, "y": 108, "shots": 0}"#;

const USERS_JSON: &str = r#"{
  "player_1": {"hp": 100, "x": 42, "y": 108},
  "highscore": 9000,
  "level": 1
}"#;

/// Deterministic 64x64 RGBA sprite (raw binary payload — proves byte-exact offsets).
fn make_sprites() -> Vec<u8> {
    let mut out = Vec::with_capacity(64 * 64 * 4);
    for y in 0..64u32 {
        for x in 0..64u32 {
            let d = ((x * x + y * y) as f32).sqrt();
            out.push((x as u8).wrapping_mul(7));
            out.push((y as u8).wrapping_mul(11));
            out.push(((d * 3.0) as u8).wrapping_add(40));
            out.push(255);
        }
    }
    out
}

/// Модуль 3 манифеста: Database/KV контейнер.
/// Таблицы = MIME-секции; byte-offset map = head index (mount показывает off/len).
pub fn build_db(path: &Path, entity: &str) -> Result<(), String> {
    let mut spatial = Vec::with_capacity(2048);
    for i in 0..2048u32 {
        spatial.push((i.wrapping_mul(2654435761)) as u8);
    }
    build_file_with_headers(
        path,
        entity,
        "DB: User State & World Data",
        "X-EML-Type: Database/KV\r\n",
        vec![
            Part::raw("users", "application/json", "users.json", USERS_JSON.as_bytes().to_vec()),
            Part::raw("items", "application/octet-stream", "spatial_index.bin", spatial),
        ],
    )?;
    Ok(())
}

/// Модуль 4 манифеста: AI/MemoryBank. Вложения = LTM; хвост = turn-дельты (KV turns).
pub fn build_mem(path: &Path, entity: &str) -> Result<(), String> {
    let ltm = "# Long-Term Knowledge\n- [Core]: Пользователь строит плоскую архитектуру на .eml\n- [Hardware]: PC, Linux, PS Vita, Mobile.\n";
    let mut vecs = Vec::with_capacity(1536 * 4);
    for i in 0..(1536u32 * 4) {
        vecs.push((i as f32 * 0.001).to_le_bytes()[0]);
    }
    build_file_with_headers(
        path,
        entity,
        "AI Agent Context & Vector Memory",
        "X-EML-Type: AI/MemoryBank\r\n",
        vec![
            Part::raw("long_term_facts", "text/markdown", "long_term_facts.md", ltm.as_bytes().to_vec()),
            Part::raw("vectors", "application/octet-stream", "vectors.idx", vecs),
        ],
    )?;
    // атомарная дозапись turn'ов (X-Turn-ID -> KV turns)
    kv::set(path, "turns", "90412", json!({"role": "user", "intent": "full_eml_architecture"}))?;
    kv::set(path, "turns", "90413", json!({"role": "assistant", "intent": "emlbox_slice0"}))?;
    Ok(())
}

/// Build the demo container at `path`. If `big`, the sprite section is 4 MiB.
pub fn build_demo(path: &Path, big: bool) -> Result<(), String> {
    let sprites = if big {
        let base = make_sprites();
        let mut v = Vec::with_capacity(4 * 1024 * 1024);
        while v.len() < 4 * 1024 * 1024 {
            v.extend_from_slice(&base);
        }
        v
    } else {
        make_sprites()
    };

    build_file(
        path,
        "game_arcade_v1",
        "Retro Arcade — one file",
        vec![
            Part::raw("view", "text/html", "view.html", VIEW_HTML.as_bytes().to_vec()),
            Part::raw("logic", "text/x-python", "logic.py", LOGIC_PY.as_bytes().to_vec()),
            Part::raw("state", "application/json", "state.json", STATE_JSON.as_bytes().to_vec()),
            Part::raw("users", "application/json", "users.json", USERS_JSON.as_bytes().to_vec()),
            Part::raw("sprites", "application/octet-stream", "sprites.bin", sprites),
        ],
    )?;

    // delta tail: in-self persistence
    kv::set(path, "users", "player_1", json!({"hp": 80, "x": 43, "y": 109}))?;
    kv::set(path, "users", "highscore", json!(12500))?;
    kv::set(path, "users", "level", json!(3))?;

    // sanity: mount + merged read
    let b = EmlBox::open(path)?;
    let p1 = kv::get(&b, "users", "player_1")?;
    let hs = kv::get(&b, "users", "highscore")?;
    assert_eq!(b.sections.len(), 5, "demo must have 5 base sections");
    assert!(p1.is_some() && hs == Some(json!(12500)), "demo KV merge failed");
    Ok(())
}
