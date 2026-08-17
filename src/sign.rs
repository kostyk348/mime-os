//! Подписи ed25519 поверх hash-chain: доверенный sync.
//!
//! Если задан EMLBOX_SEED (64 hex-символа) — каждый дельта-блок подписывается:
//! заголовки X-Writer-Key (публичный ключ) и X-Signature (подпись по телу
//! блока). verify проверяет подпись, если она есть. Подделать блок (пересобрать
//! с новым hash, чтобы пройти chain) невозможно без закрытого ключа — это
//! аутентификация писателя, а не только целостность.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

/// Активный подписывающий ключ из env, если задан EMLBOX_SEED.
pub fn active_signer() -> Option<(String, SigningKey)> {
    let seed_hex = std::env::var("EMLBOX_SEED").ok()?;
    if seed_hex.len() != 64 {
        return None;
    }
    let mut seed = [0u8; 32];
    for i in 0..32 {
        seed[i] = u8::from_str_radix(&seed_hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    let sk = SigningKey::from_bytes(&seed);
    let pk = sk.verifying_key();
    Some((hex(&pk.to_bytes()), sk))
}

/// Подпись тела (байты после blank-линии блока).
pub fn sign(sk: &SigningKey, body: &[u8]) -> String {
    hex(&sk.sign(body).to_bytes())
}

/// Проверка подписи: pubkey_hex, sig_hex по телу.
pub fn verify(pubkey_hex: &str, sig_hex: &str, body: &[u8]) -> bool {
    let pk: [u8; 32] = match dehex32(pubkey_hex) {
        Some(k) => k,
        None => return false,
    };
    let sig: [u8; 64] = match dehex64(sig_hex) {
        Some(s) => s,
        None => return false,
    };
    let pk = match VerifyingKey::from_bytes(&pk) {
        Ok(k) => k,
        Err(_) => return false,
    };
    pk.verify(body, &Signature::from_bytes(&sig)).is_ok()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn dehex32(s: &str) -> Option<[u8; 32]> {
    let s = s.trim();
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn dehex64(s: &str) -> Option<[u8; 64]> {
    let s = s.trim();
    if s.len() != 128 {
        return None;
    }
    let mut out = [0u8; 64];
    for i in 0..64 {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}
