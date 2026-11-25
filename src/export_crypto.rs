//! AES-256-GCM encryption for completed reconciliation export files.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::Engine as _;
use rand::RngCore;

use crate::error::{AppError, AppResult};

fn key_from_var(name: &str) -> Option<[u8; 32]> {
    let Ok(raw) = std::env::var(name) else {
        return None;
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(raw) else {
        tracing::warn!(%name, "export encryption key is not valid base64");
        return None;
    };
    if bytes.len() != 32 {
        tracing::warn!(
            %name,
            length = bytes.len(),
            "export encryption key must decode to 32 bytes"
        );
        return None;
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Some(key)
}

pub fn key_from_env() -> Option<[u8; 32]> {
    key_from_var("TSP_EXPORT_ENCRYPTION_KEY")
}

pub fn previous_key_from_env() -> Option<[u8; 32]> {
    key_from_var("TSP_EXPORT_ENCRYPTION_KEY_PREVIOUS")
}

pub fn decrypt_with_env(encoded: &[u8]) -> AppResult<Vec<u8>> {
    let current = key_from_env()
        .ok_or_else(|| AppError::internal("未配置有效的 TSP_EXPORT_ENCRYPTION_KEY"))?;
    match decrypt(&current, encoded) {
        Ok(plaintext) => Ok(plaintext),
        Err(primary) => {
            if let Some(previous) = previous_key_from_env() {
                decrypt(&previous, encoded).map_err(|_| primary)
            } else {
                Err(primary)
            }
        }
    }
}

pub fn encryption_required() -> bool {
    std::env::var("TSP_EXPORT_REQUIRE_ENCRYPTION")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes"))
        .unwrap_or(false)
}

pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> AppResult<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|error| AppError::internal(format!("export cipher: {error}")))?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| AppError::internal("export encryption failed"))?;
    let mut out = Vec::with_capacity(12 + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

pub fn decrypt(key: &[u8; 32], encoded: &[u8]) -> AppResult<Vec<u8>> {
    if encoded.len() < 13 {
        return Err(AppError::bad_request("加密导出文件已损坏"));
    }
    let (nonce_bytes, ciphertext) = encoded.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|error| AppError::internal(format!("export cipher: {error}")))?;
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|_| AppError::internal("导出文件解密失败"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aes_roundtrip_and_tamper_detection() {
        let key = [7u8; 32];
        let plain = b"id,vehicle_id\n1,veh\n";
        let encrypted = encrypt(&key, plain).expect("encrypt");
        assert_ne!(encrypted, plain);
        assert_eq!(decrypt(&key, &encrypted).expect("decrypt"), plain);
        let mut tampered = encrypted;
        let last = tampered.len() - 1;
        tampered[last] ^= 0x01;
        assert!(decrypt(&key, &tampered).is_err());
    }

    #[test]
    fn encryption_required_flag_parses_truthy() {
        std::env::set_var("TSP_EXPORT_REQUIRE_ENCRYPTION", "true");
        assert!(encryption_required());
        std::env::set_var("TSP_EXPORT_REQUIRE_ENCRYPTION", "false");
        assert!(!encryption_required());
        std::env::remove_var("TSP_EXPORT_REQUIRE_ENCRYPTION");
    }

    #[test]
    fn previous_key_decrypts_old_file() {
        std::env::set_var(
            "TSP_EXPORT_ENCRYPTION_KEY_PREVIOUS",
            "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=",
        );
        std::env::set_var(
            "TSP_EXPORT_ENCRYPTION_KEY",
            "YWJjZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXoxMjM0NTY=",
        );
        let old_key = [1u8; 32];
        let encrypted = encrypt(&old_key, b"old-export").expect("encrypt");
        let decrypted = decrypt_with_env(&encrypted).expect("fallback decrypt");
        assert_eq!(decrypted, b"old-export");
        std::env::remove_var("TSP_EXPORT_ENCRYPTION_KEY_PREVIOUS");
        std::env::remove_var("TSP_EXPORT_ENCRYPTION_KEY");
    }
}
