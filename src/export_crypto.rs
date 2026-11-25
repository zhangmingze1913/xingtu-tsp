//! AES-256-GCM encryption for completed reconciliation export files.
//!
//! New files carry a `TSPENC1` header plus a key id:
//!   magic(7) | key_id_len(1) | key_id | nonce(12) | ciphertext
//!
//! Legacy files without the header are decrypted by trying the configured
//! keys in order.

use std::collections::HashMap;

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::Engine as _;
use rand::RngCore;
use serde_json::Value;

use crate::error::{AppError, AppResult};

const MAGIC: &[u8; 7] = b"TSPENC1";

fn decode_key(raw: &str) -> Option<[u8; 32]> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(raw.trim())
        .ok()?;
    if bytes.len() != 32 {
        return None;
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Some(key)
}

fn key_from_var(name: &str) -> Option<[u8; 32]> {
    let raw = std::env::var(name).ok()?;
    let key = decode_key(&raw);
    if key.is_none() && !raw.trim().is_empty() {
        tracing::warn!(%name, "export encryption key must be base64 of 32 bytes");
    }
    key
}

fn key_file_is_private(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match std::fs::metadata(path) {
            Ok(meta) if meta.permissions().mode() & 0o077 != 0 => {
                tracing::warn!(
                    path = %path.display(),
                    "export key file is group/world accessible; use chmod 600 or a 0400 Secret"
                );
                return false;
            }
            Ok(_) => true,
            Err(_) => false,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        true
    }
}

fn keyring_from_file() -> Option<HashMap<String, [u8; 32]>> {
    let path = std::env::var("TSP_EXPORT_ENCRYPTION_KEYS_FILE").ok()?;
    if !key_file_is_private(std::path::Path::new(path.trim())) {
        return None;
    }
    let raw = std::fs::read_to_string(path.trim()).ok()?;
    let Value::Object(map) = serde_json::from_str::<Value>(&raw).ok()? else {
        tracing::warn!("export encryption keyring file must be a JSON object");
        return None;
    };
    let mut keys = HashMap::new();
    for (id, value) in map {
        if let Some(key) = value.as_str().and_then(decode_key) {
            keys.insert(id, key);
        }
    }
    Some(keys)
}

fn key_from_file(name: &str) -> Option<[u8; 32]> {
    let path = std::env::var(name).ok()?;
    if !key_file_is_private(std::path::Path::new(path.trim())) {
        return None;
    }
    let raw = std::fs::read_to_string(path.trim()).ok()?;
    decode_key(&raw)
}

#[derive(Clone)]
pub struct KeyRing {
    current_id: String,
    keys: HashMap<String, [u8; 32]>,
}

impl KeyRing {
    pub fn from_env() -> Option<Self> {
        let mut keys = keyring_from_file().unwrap_or_default();
        if keys.is_empty() {
            if let Ok(raw) = std::env::var("TSP_EXPORT_ENCRYPTION_KEYS") {
                if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&raw) {
                    for (id, value) in map {
                        if let Some(text) = value.as_str().and_then(decode_key) {
                            keys.insert(id, text);
                        }
                    }
                }
            }
        }
        let current_id = std::env::var("TSP_EXPORT_ENCRYPTION_KEY_ID")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "current".to_string());
        if let Some(key) = key_from_file("TSP_EXPORT_ENCRYPTION_KEY_FILE")
            .or_else(|| key_from_var("TSP_EXPORT_ENCRYPTION_KEY"))
        {
            keys.entry(current_id.clone()).or_insert(key);
        }
        if let Some(key) = key_from_file("TSP_EXPORT_ENCRYPTION_KEY_PREVIOUS_FILE")
            .or_else(|| key_from_var("TSP_EXPORT_ENCRYPTION_KEY_PREVIOUS"))
        {
            keys.entry("previous".to_string()).or_insert(key);
        }
        if keys.is_empty() {
            return None;
        }
        Some(Self { current_id, keys })
    }

    fn current(&self) -> Option<(&str, &[u8; 32])> {
        self.keys
            .get(&self.current_id)
            .map(|key| (self.current_id.as_str(), key))
    }

    fn decrypt(&self, encoded: &[u8]) -> AppResult<Vec<u8>> {
        if encoded.starts_with(MAGIC) {
            if encoded.len() < 8 {
                return Err(AppError::bad_request("加密导出文件已损坏"));
            }
            let id_len = encoded[7] as usize;
            if encoded.len() < 8 + id_len + 13 {
                return Err(AppError::bad_request("加密导出文件已损坏"));
            }
            let id = std::str::from_utf8(&encoded[8..8 + id_len])
                .map_err(|_| AppError::bad_request("密钥 ID 无效"))?;
            let key = self
                .keys
                .get(id)
                .ok_or_else(|| AppError::internal(format!("缺少密钥 ID {id}")))?;
            return decrypt_raw(key, &encoded[8 + id_len..]);
        }
        for key in self.keys.values() {
            if let Ok(plaintext) = decrypt_raw(key, encoded) {
                return Ok(plaintext);
            }
        }
        Err(AppError::internal("导出文件解密失败：没有匹配的密钥"))
    }
}

pub fn encryption_configured() -> bool {
    KeyRing::from_env()
        .and_then(|ring| ring.current().map(|_| ()))
        .is_some()
}

pub fn key_from_env() -> Option<[u8; 32]> {
    key_from_var("TSP_EXPORT_ENCRYPTION_KEY")
}

pub fn encryption_required() -> bool {
    std::env::var("TSP_EXPORT_REQUIRE_ENCRYPTION")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes"))
        .unwrap_or(false)
}

pub fn encrypt_with_env(plaintext: &[u8]) -> AppResult<Vec<u8>> {
    let ring = KeyRing::from_env().ok_or_else(|| AppError::internal("未配置导出加密密钥"))?;
    let (id, key) = ring
        .current()
        .ok_or_else(|| AppError::internal("当前导出密钥不存在"))?;
    let id = id.as_bytes();
    if id.len() > u8::MAX as usize {
        return Err(AppError::internal("导出密钥 ID 过长"));
    }
    let body = encrypt_raw(key, plaintext)?;
    let mut out = Vec::with_capacity(8 + id.len() + body.len());
    out.extend_from_slice(MAGIC);
    out.push(id.len() as u8);
    out.extend_from_slice(id);
    out.extend_from_slice(&body);
    Ok(out)
}

pub fn decrypt_with_env(encoded: &[u8]) -> AppResult<Vec<u8>> {
    let ring = KeyRing::from_env().ok_or_else(|| AppError::internal("未配置导出加密密钥"))?;
    ring.decrypt(encoded)
}

pub fn encrypt_raw(key: &[u8; 32], plaintext: &[u8]) -> AppResult<Vec<u8>> {
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

pub fn decrypt_raw(key: &[u8; 32], encoded: &[u8]) -> AppResult<Vec<u8>> {
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

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn clear_env() {
        for name in [
            "TSP_EXPORT_ENCRYPTION_KEYS",
            "TSP_EXPORT_ENCRYPTION_KEY_ID",
            "TSP_EXPORT_ENCRYPTION_KEY",
            "TSP_EXPORT_ENCRYPTION_KEY_PREVIOUS",
        ] {
            std::env::remove_var(name);
        }
    }

    #[test]
    fn raw_roundtrip_and_tamper_detection() {
        let key = [7u8; 32];
        let plain = b"id,vehicle_id\n1,veh\n";
        let encrypted = encrypt_raw(&key, plain).expect("encrypt");
        assert_eq!(decrypt_raw(&key, &encrypted).expect("decrypt"), plain);
        let mut tampered = encrypted;
        let last = tampered.len() - 1;
        tampered[last] ^= 0x01;
        assert!(decrypt_raw(&key, &tampered).is_err());
    }

    #[test]
    fn keyring_writes_key_id_and_decrypts_by_id() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();
        let k1 = base64::engine::general_purpose::STANDARD.encode([1u8; 32]);
        let k2 = base64::engine::general_purpose::STANDARD.encode([2u8; 32]);
        std::env::set_var(
            "TSP_EXPORT_ENCRYPTION_KEYS",
            format!("{{\"k1\":\"{k1}\",\"k2\":\"{k2}\"}}"),
        );
        std::env::set_var("TSP_EXPORT_ENCRYPTION_KEY_ID", "k2");
        let encrypted = encrypt_with_env(b"payload").expect("encrypt");
        assert!(encrypted.starts_with(MAGIC));
        let id_len = encrypted[7] as usize;
        assert_eq!(&encrypted[8..8 + id_len], b"k2");
        assert_eq!(decrypt_with_env(&encrypted).expect("decrypt"), b"payload");
        clear_env();
    }

    #[test]
    fn legacy_file_falls_back_to_configured_key() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();
        let key = [9u8; 32];
        let legacy = encrypt_raw(&key, b"legacy").expect("encrypt");
        std::env::set_var(
            "TSP_EXPORT_ENCRYPTION_KEY",
            base64::engine::general_purpose::STANDARD.encode(key),
        );
        assert_eq!(decrypt_with_env(&legacy).expect("decrypt"), b"legacy");
        clear_env();
    }

    #[test]
    fn encryption_required_flag_parses_truthy() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("TSP_EXPORT_REQUIRE_ENCRYPTION", "true");
        assert!(encryption_required());
        std::env::set_var("TSP_EXPORT_REQUIRE_ENCRYPTION", "false");
        assert!(!encryption_required());
        std::env::remove_var("TSP_EXPORT_REQUIRE_ENCRYPTION");
    }

    #[test]
    fn keyring_and_single_key_load_from_files() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();
        let dir = std::env::temp_dir().join(format!("tsp-keyring-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("dir");
        let k1 = base64::engine::general_purpose::STANDARD.encode([1u8; 32]);
        let k2 = base64::engine::general_purpose::STANDARD.encode([2u8; 32]);
        let ring_path = dir.join("keys.json");
        std::fs::write(&ring_path, format!("{{\"k1\":\"{k1}\",\"k2\":\"{k2}\"}}"))
            .expect("write ring");
        std::env::set_var("TSP_EXPORT_ENCRYPTION_KEYS_FILE", &ring_path);
        std::env::set_var("TSP_EXPORT_ENCRYPTION_KEY_ID", "k2");
        let encrypted = encrypt_with_env(b"file-keyring").expect("encrypt");
        assert_eq!(
            decrypt_with_env(&encrypted).expect("decrypt"),
            b"file-keyring"
        );
        clear_env();
        let single_path = dir.join("key.txt");
        std::fs::write(
            &single_path,
            base64::engine::general_purpose::STANDARD.encode([3u8; 32]),
        )
        .expect("write key");
        std::env::set_var("TSP_EXPORT_ENCRYPTION_KEY_FILE", &single_path);
        let encrypted = encrypt_with_env(b"single-file").expect("encrypt single");
        assert_eq!(
            decrypt_with_env(&encrypted).expect("decrypt single"),
            b"single-file"
        );
        clear_env();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn key_file_permissions_are_enforced() {
        use std::os::unix::fs::PermissionsExt;
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();
        let dir = std::env::temp_dir().join(format!("tsp-keyperm-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("key");
        std::fs::write(
            &path,
            base64::engine::general_purpose::STANDARD.encode([4u8; 32]),
        )
        .expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
        assert!(key_file_is_private(&path));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        assert!(!key_file_is_private(&path));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
