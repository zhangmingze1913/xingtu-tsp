//! RS256 key ring and JWKS publication.
//!
//! The active private key signs new tokens. Previous public keys listed in
//! `TSP_JWT_PREVIOUS_KEYS` (`kid=path,kid=path`) remain accepted and published
//! in JWKS until the configured retention window ends (the operator removes
//! them from the list).

use std::{sync::Arc, sync::OnceLock};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use jsonwebtoken::{decode_header, Algorithm, DecodingKey, EncodingKey};
use rsa::{
    pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey, LineEnding},
    traits::PublicKeyParts,
    RsaPrivateKey, RsaPublicKey,
};
use serde_json::{json, Value};

use crate::{config::Config, error::AppError};

#[derive(Clone)]
pub struct JwtKeys {
    pub(crate) encoding: EncodingKey,
    keys: Vec<KeyEntry>,
    pub(crate) kid: String,
    pub(crate) issuer: String,
    pub(crate) audience: String,
    pub(crate) access_ttl_minutes: i64,
}

#[derive(Clone)]
struct KeyEntry {
    kid: String,
    decoding: DecodingKey,
    n: String,
    e: String,
}

static DEV_KEYS: OnceLock<Arc<JwtKeys>> = OnceLock::new();

impl JwtKeys {
    pub(crate) fn from_config(config: &Config) -> Result<Arc<Self>, AppError> {
        if let Some(private_pem) = config.jwt_private_key.as_deref() {
            let public_pem = match config.jwt_public_key.as_deref() {
                Some(value) => value.to_string(),
                None => derive_public_pem(private_pem)?,
            };
            let mut previous = Vec::new();
            for entry in &config.jwt_previous_keys {
                let (kid, path) = entry.split_once('=').ok_or_else(|| {
                    AppError::internal("TSP_JWT_PREVIOUS_KEYS must be kid=path entries")
                })?;
                let pem = std::fs::read_to_string(path).map_err(|error| {
                    tracing::error!(%error, %path, "failed to read previous JWT public key");
                    AppError::internal("failed to read previous JWT public key")
                })?;
                previous.push((kid.to_string(), pem));
            }
            return build_keys(
                private_pem,
                &public_pem,
                config.jwt_key_id.clone(),
                config.jwt_issuer.clone(),
                config.jwt_audience.clone(),
                config.jwt_access_ttl_minutes.max(1),
                previous,
            )
            .map(Arc::new);
        }

        tracing::warn!(
            "TSP_JWT_PRIVATE_KEY is not configured; using process-local RS256 dev key (tokens die on restart)"
        );
        let keys = DEV_KEYS.get_or_init(|| {
            let (private_pem, public_pem) = generate_pem().expect("dev RS256 key generation");
            Arc::new(
                build_keys(
                    &private_pem,
                    &public_pem,
                    config.jwt_key_id.clone(),
                    config.jwt_issuer.clone(),
                    config.jwt_audience.clone(),
                    config.jwt_access_ttl_minutes.max(1),
                    Vec::new(),
                )
                .expect("dev RS256 key setup"),
            )
        });
        Ok(keys.clone())
    }

    pub(crate) fn validation(&self) -> jsonwebtoken::Validation {
        let mut validation = jsonwebtoken::Validation::new(Algorithm::RS256);
        validation.set_issuer(&[self.issuer.as_str()]);
        validation.set_audience(&[self.audience.as_str()]);
        validation.validate_exp = true;
        validation.leeway = 5;
        validation
    }

    pub(crate) fn decoding_for(&self, token: &str) -> Result<&DecodingKey, AppError> {
        let kid = decode_header(token).ok().and_then(|header| header.kid);
        if let Some(kid) = kid {
            if let Some(entry) = self.keys.iter().find(|entry| entry.kid == kid) {
                return Ok(&entry.decoding);
            }
        }
        self.keys
            .first()
            .map(|entry| &entry.decoding)
            .ok_or_else(|| AppError::internal("no RS256 verification key configured"))
    }

    pub(crate) fn jwks(&self) -> Value {
        let keys: Vec<Value> = self
            .keys
            .iter()
            .map(|entry| {
                json!({
                    "kty": "RSA",
                    "use": "sig",
                    "alg": "RS256",
                    "kid": entry.kid,
                    "n": entry.n,
                    "e": entry.e
                })
            })
            .collect();
        json!({ "keys": keys })
    }
}

#[allow(clippy::too_many_arguments)]
fn build_keys(
    private_pem: &str,
    public_pem: &str,
    kid: String,
    issuer: String,
    audience: String,
    access_ttl_minutes: i64,
    previous: Vec<(String, String)>,
) -> Result<JwtKeys, AppError> {
    let encoding = EncodingKey::from_rsa_pem(private_pem.as_bytes())
        .map_err(|_| AppError::internal("invalid RS256 private key"))?;
    let mut keys = vec![parse_key_entry(&kid, public_pem)?];
    for (previous_kid, pem) in previous {
        keys.push(parse_key_entry(&previous_kid, &pem)?);
    }
    Ok(JwtKeys {
        encoding,
        keys,
        kid,
        issuer,
        audience,
        access_ttl_minutes,
    })
}

fn parse_key_entry(kid: &str, public_pem: &str) -> Result<KeyEntry, AppError> {
    let decoding = DecodingKey::from_rsa_pem(public_pem.as_bytes())
        .map_err(|_| AppError::internal("invalid RS256 public key"))?;
    let public = RsaPublicKey::from_public_key_pem(public_pem)
        .map_err(|_| AppError::internal("invalid RSA public key"))?;
    Ok(KeyEntry {
        kid: kid.to_string(),
        decoding,
        n: URL_SAFE_NO_PAD.encode(public.n().to_bytes_be()),
        e: URL_SAFE_NO_PAD.encode(public.e().to_bytes_be()),
    })
}

fn derive_public_pem(private_pem: &str) -> Result<String, AppError> {
    let private = RsaPrivateKey::from_pkcs8_pem(private_pem)
        .map_err(|_| AppError::internal("invalid RSA private key"))?;
    private
        .to_public_key()
        .to_public_key_pem(LineEnding::LF)
        .map_err(|_| AppError::internal("failed to derive RSA public key"))
}

fn generate_pem() -> Result<(String, String), AppError> {
    let mut rng = rand::rngs::OsRng;
    let private = RsaPrivateKey::new(&mut rng, 2048)
        .map_err(|_| AppError::internal("failed to generate RS256 key"))?;
    let private_pem = private
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|_| AppError::internal("failed to encode RS256 private key"))?
        .to_string();
    let public_pem = private
        .to_public_key()
        .to_public_key_pem(LineEnding::LF)
        .map_err(|_| AppError::internal("failed to encode RSA public key"))?;
    Ok((private_pem, public_pem))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, Header};

    #[test]
    fn key_ring_publishes_active_and_previous_keys() {
        let (private, public) = generate_pem().expect("rsa");
        let (_, previous_public) = generate_pem().expect("rsa");
        let keys = build_keys(
            &private,
            &public,
            "new-kid".to_string(),
            "issuer".to_string(),
            "audience".to_string(),
            15,
            vec![("old-kid".to_string(), previous_public)],
        )
        .expect("keys");
        assert_eq!(keys.jwks()["keys"].as_array().unwrap().len(), 2);
        assert!(keys.decoding_for("no-kid").is_ok());

        let claims = serde_json::json!({"sub": "u", "exp": 4_000_000_000u64});
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("new-kid".to_string());
        let token = encode(&header, &claims, &keys.encoding).expect("token");
        let selected = keys.decoding_for(&token).expect("active decode");
        let mut validation = keys.validation();
        validation.validate_aud = false;
        let decoded = jsonwebtoken::decode::<serde_json::Value>(&token, selected, &validation)
            .expect("decode");
        assert_eq!(decoded.claims["sub"], "u");
    }
}
