//! X-Encoding для секций: deflate (flate2) и aes-256-gcm (RustCrypto).
//!
//! В контейнере секция хранится ЗАКОДИРОВАННОЙ (X-Encoding: deflate|aes),
//! off/len/hash считаются по закодированным байтам; читатель декодирует на
//! лету. verify хэширует закодированное — целостность покрывает шифрование.
//!
//! Ключ: env EMLBOX_KEY (64 hex-символа) или EMLBOX_PASS (sha256 пароля).

use crate::format::ENC_RAW;

pub const ENC_DEFLATE: &str = "deflate";
pub const ENC_AES: &str = "aes";

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};

pub fn encode(enc: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    match enc {
        ENC_RAW => Ok(data.to_vec()),
        ENC_DEFLATE => compress(data),
        ENC_AES => encrypt(data),
        other => Err(format!("unknown encoding: {other}")),
    }
}

pub fn decode(enc: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    match enc {
        ENC_RAW => Ok(data.to_vec()),
        ENC_DEFLATE => decompress(data),
        ENC_AES => decrypt(data),
        other => Err(format!("unknown encoding: {other}")),
    }
}

fn compress(data: &[u8]) -> Result<Vec<u8>, String> {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;
    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).map_err(|e| e.to_string())?;
    e.finish().map_err(|e| e.to_string())
}

fn decompress(data: &[u8]) -> Result<Vec<u8>, String> {
    use flate2::read::ZlibDecoder;
    use std::io::Read;
    let mut d = ZlibDecoder::new(data);
    let mut out = Vec::new();
    d.read_to_end(&mut out).map_err(|e| format!("deflate: {e}"))?;
    Ok(out)
}

fn key() -> Result<[u8; 32], String> {
    if let Ok(k) = std::env::var("EMLBOX_KEY") {
        let k = k.trim();
        if k.len() != 64 {
            return Err("EMLBOX_KEY must be 64 hex chars".into());
        }
        let mut out = [0u8; 32];
        for i in 0..32 {
            out[i] = u8::from_str_radix(&k[i * 2..i * 2 + 2], 16).map_err(|_| "bad hex in EMLBOX_KEY")?;
        }
        return Ok(out);
    }
    if let Ok(p) = std::env::var("EMLBOX_PASS") {
        use sha2::{Digest, Sha256};
        return Ok(Sha256::digest(p.as_bytes()).into());
    }
    Err("aes: set EMLBOX_KEY (64 hex) or EMLBOX_PASS".into())
}

/// Активный ключ из env, если задан (EMLBOX_KEY или EMLBOX_PASS).
/// None — шифрование не включено (сырые дельты, обратная совместимость).
pub fn active_key() -> Result<Option<[u8; 32]>, String> {
    if std::env::var("EMLBOX_KEY").is_ok() || std::env::var("EMLBOX_PASS").is_ok() {
        Ok(Some(key()?))
    } else {
        Ok(None)
    }
}

fn encrypt(data: &[u8]) -> Result<Vec<u8>, String> {
    encrypt_key(&key()?, data)
}

fn decrypt(data: &[u8]) -> Result<Vec<u8>, String> {
    decrypt_key(&key()?, data)
}

/// Шифрование с явным ключом (без глобального env) — для API/тестов.
pub fn encrypt_key(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>, String> {
    use rand::RngCore;
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), data)
        .map_err(|e| format!("aes encrypt: {e}"))?;
    let mut out = Vec::with_capacity(12 + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Расшифровка с явным ключом.
pub fn decrypt_key(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 12 {
        return Err("aes: data too short".into());
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
    let (nonce, ct) = data.split_at(12);
    cipher
        .decrypt(Nonce::from_slice(nonce), ct)
        .map_err(|_| "aes decrypt failed (wrong key?)".to_string())
}
