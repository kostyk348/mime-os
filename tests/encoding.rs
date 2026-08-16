//! X-Encoding: deflate + aes-256-gcm для секций.

use emlbox::encoding;
use emlbox::kv;
use emlbox::verify;
use emlbox::writer::{build_file, Part};
use serde_json::json;
use std::path::PathBuf;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("emlbox_enc_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.join("c.eml")
}

#[test]
fn deflate_roundtrip_and_smaller() {
    let payload = b"hello world hello world hello world hello world hello world hello world hello world";
    let enc = encoding::encode("deflate", payload).unwrap();
    assert!(enc.len() < payload.len(), "compressed {} < {}", enc.len(), payload.len());
    let dec = encoding::decode("deflate", &enc).unwrap();
    assert_eq!(dec, payload);
}

#[test]
fn aes_roundtrip_requires_key() {
    // без ключа — ошибка
    std::env::remove_var("EMLBOX_KEY");
    std::env::remove_var("EMLBOX_PASS");
    assert!(encoding::encode("aes", b"x").is_err());

    let key = [7u8; 32];
    let enc = encoding::encrypt_key(&key, b"secret data").unwrap();
    assert_ne!(enc, b"secret data");
    let dec = encoding::decrypt_key(&key, &enc).unwrap();
    assert_eq!(dec, b"secret data");

    // неверный ключ — не расшифруется
    let wrong = [8u8; 32];
    assert!(encoding::decrypt_key(&wrong, &enc).is_err());
}

#[test]
fn encrypted_kv_database() {
    std::env::set_var("EMLBOX_PASS", "kv-secret");
    let p = tmp("kv");
    build_file(
        &p,
        "bank@system.local",
        "bank",
        vec![Part {
            id: "state".into(),
            ct: "application/json".into(),
            name: "state.json".into(),
            enc: "aes".into(),
            data: br#"{"balance":0}"#.to_vec(),
        }],
    )
    .unwrap();
    // на диске — шифр
    let raw = std::fs::read(&p).unwrap();
    let rawstr = String::from_utf8_lossy(&raw);
    assert!(!rawstr.contains("balance"), "база на диске не должна содержать plaintext");

    // KV работает сквозь шифрование
    kv::set(&p, "state", "balance", json!(1000)).unwrap();
    let b = emlbox::reader::EmlBox::open(&p).unwrap();
    assert_eq!(kv::get(&b, "state", "balance").unwrap(), Some(json!(1000)));

    // verify целостен (хэширует зашифрованные байты)
    assert!(verify::verify(&p).unwrap().is_empty());

    // без ключа база не читается
    std::env::remove_var("EMLBOX_PASS");
    let b = emlbox::reader::EmlBox::open(&p).unwrap();
    assert!(kv::get(&b, "state", "balance").is_err(), "без ключа — ошибка декодирования");
}

#[test]
fn deflate_section_readable() {
    let p = tmp("deflate");
    let body = "x".repeat(5000);
    build_file(
        &p,
        "docs@system.local",
        "docs",
        vec![Part {
            id: "text".into(),
            ct: "text/plain".into(),
            name: "t.txt".into(),
            enc: "deflate".into(),
            data: body.as_bytes().to_vec(),
        }],
    )
    .unwrap();
    let b = emlbox::reader::EmlBox::open(&p).unwrap();
    assert_eq!(String::from_utf8_lossy(&b.section("text").unwrap()).to_string(), body);
    // файл существенно меньше 5000 байт тела
    let size = std::fs::metadata(&p).unwrap().len();
    assert!(size < 2000, "сжатая секция: file={size}");
}
