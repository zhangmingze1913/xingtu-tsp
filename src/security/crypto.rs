//! Field-level authenticated encryption for PII at rest.

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CryptoError {
    #[error("PII key must be 32 bytes")]
    KeyLength,
    #[error("invalid PII key encoding")]
    KeyEncoding,
    #[error("PII encryption failed")]
    Encrypt,
    #[error("PII decryption failed")]
    Decrypt,
    #[error("PII ciphertext is malformed")]
    Ciphertext,
}

pub struct FieldCipher {
    cipher: Aes256Gcm,
}

impl FieldCipher {
    pub fn new(key: &[u8; 32]) -> Self {
        Self {
            cipher: Aes256Gcm::new_from_slice(key).expect("32-byte key"),
        }
    }

    /// Key format: 64 hex characters (32 bytes).
    pub fn from_hex(value: &str) -> Result<Self, CryptoError> {
        let value = value.trim();
        if value.len() != 64 {
            return Err(CryptoError::KeyLength);
        }
        let mut key = [0u8; 32];
        for index in 0..32 {
            let byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
                .map_err(|_| CryptoError::KeyEncoding)?;
            key[index] = byte;
        }
        Ok(Self::new(&key))
    }

    pub fn from_file(path: impl AsRef<std::path::Path>) -> Result<Self, CryptoError> {
        let value = std::fs::read_to_string(path).map_err(|_| CryptoError::KeyEncoding)?;
        Self::from_hex(&value)
    }

    pub fn from_env() -> Result<Self, CryptoError> {
        if let Ok(value) = std::env::var("TSP_PII_KEY") {
            return Self::from_hex(&value);
        }
        let path = std::env::var("TSP_PII_KEY_FILE").map_err(|_| CryptoError::KeyEncoding)?;
        Self::from_file(path)
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<String, CryptoError> {
        let mut nonce = [0u8; 12];
        OsRng.fill_bytes(&mut nonce);
        let ciphertext = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext.as_bytes())
            .map_err(|_| CryptoError::Encrypt)?;
        let mut envelope = Vec::with_capacity(12 + ciphertext.len());
        envelope.extend_from_slice(&nonce);
        envelope.extend_from_slice(&ciphertext);
        Ok(URL_SAFE_NO_PAD.encode(envelope))
    }

    pub fn decrypt(&self, envelope: &str) -> Result<String, CryptoError> {
        let raw = URL_SAFE_NO_PAD
            .decode(envelope)
            .map_err(|_| CryptoError::Ciphertext)?;
        if raw.len() < 12 + 16 {
            return Err(CryptoError::Ciphertext);
        }
        let (nonce, ciphertext) = raw.split_at(12);
        let plaintext = self
            .cipher
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .map_err(|_| CryptoError::Decrypt)?;
        String::from_utf8(plaintext).map_err(|_| CryptoError::Decrypt)
    }
}

/// Boundary for local keys, HSM or KMS-backed providers.
pub trait KeyProvider: Send + Sync {
    fn provider_name(&self) -> &'static str;
    fn encrypt(&self, plaintext: &str) -> Result<String, CryptoError>;
    fn decrypt(&self, envelope: &str) -> Result<String, CryptoError>;
}

/// Synchronous transport for a KMS/HSM-backed provider.
pub trait KmsTransport: Send + Sync {
    fn encrypt(&self, plaintext: &str) -> Result<String, CryptoError>;
    fn decrypt(&self, envelope: &str) -> Result<String, CryptoError>;
}

/// KeyProvider backed by an external KMS/HSM transport.
pub struct KmsKeyProvider<T: KmsTransport> {
    name: &'static str,
    transport: T,
}

impl<T: KmsTransport> KmsKeyProvider<T> {
    pub fn new(name: &'static str, transport: T) -> Self {
        Self { name, transport }
    }
}

impl<T: KmsTransport> KeyProvider for KmsKeyProvider<T> {
    fn provider_name(&self) -> &'static str {
        self.name
    }

    fn encrypt(&self, plaintext: &str) -> Result<String, CryptoError> {
        self.transport.encrypt(plaintext)
    }

    fn decrypt(&self, envelope: &str) -> Result<String, CryptoError> {
        self.transport.decrypt(envelope)
    }
}

/// Minimal HTTP KMS transport. The contract is intentionally small:
/// POST {endpoint} with {"action":"encrypt|decrypt","value":"..."} and read {"value":"..."}.
pub struct HttpKmsTransport {
    endpoint: String,
    client: reqwest::blocking::Client,
    token: Option<String>,
}

impl HttpKmsTransport {
    pub fn new(endpoint: impl Into<String>, token: Option<String>) -> Result<Self, CryptoError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|_| CryptoError::Encrypt)?;
        Ok(Self {
            endpoint: endpoint.into(),
            client,
            token,
        })
    }

    fn call(&self, action: &str, value: &str) -> Result<String, CryptoError> {
        let mut request = self
            .client
            .post(&self.endpoint)
            .json(&serde_json::json!({ "action": action, "value": value }));
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request.send().map_err(|_| CryptoError::Encrypt)?;
        if !response.status().is_success() {
            return Err(CryptoError::Encrypt);
        }
        let body: serde_json::Value = response.json().map_err(|_| CryptoError::Encrypt)?;
        body.get("value")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or(CryptoError::Encrypt)
    }
}

impl KmsTransport for HttpKmsTransport {
    fn encrypt(&self, plaintext: &str) -> Result<String, CryptoError> {
        self.call("encrypt", plaintext)
    }

    fn decrypt(&self, envelope: &str) -> Result<String, CryptoError> {
        self.call("decrypt", envelope)
    }
}

/// Build the configured provider: HTTP KMS when TSP_KMS_ENDPOINT is set,
/// otherwise the local key ring from TSP_PII_KEY/TSP_PII_KEY_FILE.
pub fn provider_from_env() -> Option<Arc<dyn KeyProvider>> {
    if let Ok(endpoint) = std::env::var("TSP_KMS_ENDPOINT") {
        let token = std::env::var("TSP_KMS_TOKEN").ok();
        let transport = HttpKmsTransport::new(endpoint, token).ok()?;
        return Some(Arc::new(KmsKeyProvider::new("http-kms", transport)));
    }
    KeyRing::from_env()
        .ok()
        .map(|ring| Arc::new(ring) as Arc<dyn KeyProvider>)
}

/// In-memory key usage counters (no plaintext, no key material).
#[derive(Debug, Default)]
pub struct KeyUsageStats {
    encryptions: AtomicU64,
    decryptions: AtomicU64,
    failures: AtomicU64,
}

impl KeyUsageStats {
    pub fn encryptions(&self) -> u64 {
        self.encryptions.load(Ordering::Relaxed)
    }

    pub fn decryptions(&self) -> u64 {
        self.decryptions.load(Ordering::Relaxed)
    }

    pub fn failures(&self) -> u64 {
        self.failures.load(Ordering::Relaxed)
    }
}

/// Primary key plus retired keys for decryption during rotation.
pub struct KeyRing {
    primary: [u8; 32],
    previous: Vec<[u8; 32]>,
    stats: Arc<KeyUsageStats>,
}

impl KeyProvider for KeyRing {
    fn provider_name(&self) -> &'static str {
        "local-keyring"
    }

    fn encrypt(&self, plaintext: &str) -> Result<String, CryptoError> {
        KeyRing::encrypt(self, plaintext)
    }

    fn decrypt(&self, envelope: &str) -> Result<String, CryptoError> {
        KeyRing::decrypt(self, envelope)
    }
}

impl KeyRing {
    pub fn new(primary: [u8; 32], previous: Vec<[u8; 32]>) -> Self {
        Self {
            primary,
            previous,
            stats: Arc::new(KeyUsageStats::default()),
        }
    }

    pub fn stats(&self) -> Arc<KeyUsageStats> {
        Arc::clone(&self.stats)
    }

    pub fn from_env() -> Result<Self, CryptoError> {
        let primary =
            parse_hex_key(&std::env::var("TSP_PII_KEY").map_err(|_| CryptoError::KeyEncoding)?)?;
        let previous = std::env::var("TSP_PII_PREVIOUS_KEYS")
            .unwrap_or_default()
            .split(',')
            .filter(|value| !value.trim().is_empty())
            .map(parse_hex_key)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            primary,
            previous,
            stats: Arc::new(KeyUsageStats::default()),
        })
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<String, CryptoError> {
        self.stats.encryptions.fetch_add(1, Ordering::Relaxed);
        FieldCipher::new(&self.primary).encrypt(plaintext)
    }

    pub fn decrypt(&self, envelope: &str) -> Result<String, CryptoError> {
        let primary = FieldCipher::new(&self.primary);
        if let Ok(value) = primary.decrypt(envelope) {
            self.stats.decryptions.fetch_add(1, Ordering::Relaxed);
            return Ok(value);
        }
        for key in &self.previous {
            if let Ok(value) = FieldCipher::new(key).decrypt(envelope) {
                self.stats.decryptions.fetch_add(1, Ordering::Relaxed);
                return Ok(value);
            }
        }
        self.stats.failures.fetch_add(1, Ordering::Relaxed);
        Err(CryptoError::Decrypt)
    }
}

fn parse_hex_key(value: &str) -> Result<[u8; 32], CryptoError> {
    let value = value.trim();
    if value.len() != 64 {
        return Err(CryptoError::KeyLength);
    }
    let mut key = [0u8; 32];
    for index in 0..32 {
        key[index] = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| CryptoError::KeyEncoding)?;
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cipher() -> FieldCipher {
        FieldCipher::from_hex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f")
            .expect("key")
    }

    #[test]
    fn aes_gcm_roundtrip_and_wrong_key() {
        let cipher = cipher();
        let encrypted = cipher.encrypt("LSVAA123456789012").expect("encrypt");
        assert_eq!(cipher.decrypt(&encrypted).unwrap(), "LSVAA123456789012");
        assert_ne!(encrypted, "LSVAA123456789012");

        let other = FieldCipher::from_hex(
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        )
        .unwrap();
        assert_eq!(other.decrypt(&encrypted), Err(CryptoError::Decrypt));
        assert_eq!(
            FieldCipher::from_hex("00").err(),
            Some(CryptoError::KeyLength)
        );

        let old = [0x11u8; 32];
        let new = [0x22u8; 32];
        let old_ring = KeyRing::new(old, vec![]);
        let rotated = KeyRing::new(new, vec![old]);
        let envelope = old_ring.encrypt("rotation").unwrap();
        assert_eq!(rotated.decrypt(&envelope).unwrap(), "rotation");
        assert!(old_ring.decrypt(&rotated.encrypt("new").unwrap()).is_err());

        let provider: &dyn KeyProvider = &rotated;
        assert_eq!(provider.provider_name(), "local-keyring");
        assert_eq!(
            provider
                .decrypt(&provider.encrypt("trait").unwrap())
                .unwrap(),
            "trait"
        );
    }
}
