//! Vault KV v2 -> export keyring file synchronizer.
//!
//! Vault path example: `secret/data/xingtu/export` (KV v2 mounts `data`).
//! The secret payload must be `{"data":{"data":{"k1":"<base64 32B>", ...}}}`.
//! The resulting JSON object is written atomically to the keyring file consumed
//! by `export_crypto`.

use std::{path::Path, sync::atomic::AtomicU64};

use base64::Engine as _;
use serde_json::Value;

use crate::error::{AppError, AppResult};

pub static VAULT_SYNC_OK: AtomicU64 = AtomicU64::new(0);
pub static VAULT_SYNC_FAILED: AtomicU64 = AtomicU64::new(0);
pub static VAULT_LOGIN_OK: AtomicU64 = AtomicU64::new(0);
pub static VAULT_LOGIN_FAILED: AtomicU64 = AtomicU64::new(0);

pub fn token_from_env() -> Option<String> {
    if let Ok(token) = std::env::var("TSP_VAULT_TOKEN") {
        if !token.trim().is_empty() {
            return Some(token.trim().to_string());
        }
    }
    let path = std::env::var("TSP_VAULT_TOKEN_FILE").ok()?;
    std::fs::read_to_string(path.trim())
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn secret_id_from_env() -> Option<String> {
    if let Ok(secret) = std::env::var("TSP_VAULT_SECRET_ID") {
        if !secret.trim().is_empty() {
            return Some(secret.trim().to_string());
        }
    }
    let path = std::env::var("TSP_VAULT_SECRET_ID_FILE").ok()?;
    std::fs::read_to_string(path.trim())
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub async fn login_approle(
    addr: &str,
    role_id: &str,
    secret_id: &str,
    namespace: Option<&str>,
) -> AppResult<String> {
    let base = addr.trim_end_matches('/');
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|error| AppError::internal(format!("vault client: {error}")))?;
    let mut request = client
        .post(format!("{base}/v1/auth/approle/login"))
        .json(&serde_json::json!({"role_id": role_id, "secret_id": secret_id}));
    if let Some(namespace) = namespace.filter(|value| !value.trim().is_empty()) {
        request = request.header("X-Vault-Namespace", namespace);
    }
    let response = request
        .send()
        .await
        .map_err(|error| AppError::internal(format!("vault login: {error}")))?;
    if !response.status().is_success() {
        return Err(AppError::internal(format!(
            "vault login HTTP {}",
            response.status()
        )));
    }
    let value: Value = response
        .json()
        .await
        .map_err(|error| AppError::internal(format!("vault login json: {error}")))?;
    value
        .get("auth")
        .and_then(|value| value.get("client_token"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| AppError::internal("vault login missing client_token"))
}

/// Resolve a Vault token from the environment, a token file, or AppRole.
pub async fn resolve_token(addr: &str, namespace: Option<&str>) -> AppResult<String> {
    if let Some(token) = token_from_env() {
        return Ok(token);
    }
    let role_id = std::env::var("TSP_VAULT_ROLE_ID")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let secret_id = secret_id_from_env();
    match (role_id, secret_id) {
        (Some(role_id), Some(secret_id)) => {
            match login_approle(addr, role_id.trim(), &secret_id, namespace).await {
                Ok(token) => {
                    VAULT_LOGIN_OK.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    Ok(token)
                }
                Err(error) => {
                    VAULT_LOGIN_FAILED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    Err(error)
                }
            }
        }
        _ => Err(AppError::internal(
            "no Vault token or AppRole credentials configured",
        )),
    }
}

pub async fn sync_once(
    addr: &str,
    token: &str,
    path: &str,
    namespace: Option<&str>,
    target: &Path,
) -> AppResult<usize> {
    let base = addr.trim_end_matches('/');
    let url = format!("{base}/v1/{}", path.trim_start_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|error| AppError::internal(format!("vault client: {error}")))?;
    let mut request = client.get(&url).header("X-Vault-Token", token);
    if let Some(namespace) = namespace.filter(|value| !value.trim().is_empty()) {
        request = request.header("X-Vault-Namespace", namespace);
    }
    let response = request
        .send()
        .await
        .map_err(|error| AppError::internal(format!("vault request: {error}")))?;
    if !response.status().is_success() {
        return Err(AppError::internal(format!(
            "vault HTTP {}",
            response.status()
        )));
    }
    let value: Value = response
        .json()
        .await
        .map_err(|error| AppError::internal(format!("vault json: {error}")))?;
    let keys = value
        .get("data")
        .and_then(|value| value.get("data"))
        .and_then(Value::as_object)
        .ok_or_else(|| AppError::internal("vault secret missing data.data"))?;
    let mut out = serde_json::Map::new();
    for (id, value) in keys {
        let Some(encoded) = value.as_str() else {
            continue;
        };
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded) else {
            tracing::warn!(%id, "vault export key is not base64");
            continue;
        };
        if bytes.len() != 32 {
            tracing::warn!(%id, "vault export key is not 32 bytes");
            continue;
        }
        out.insert(id.clone(), Value::String(encoded.to_string()));
    }
    if out.is_empty() {
        return Err(AppError::internal(
            "vault secret contains no valid export keys",
        ));
    }
    let encoded = serde_json::to_vec(&Value::Object(out))
        .map_err(|error| AppError::internal(format!("vault encode: {error}")))?;
    let tmp = target.with_extension("json.tmp");
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| AppError::internal(format!("vault key dir: {error}")))?;
    }
    tokio::fs::write(&tmp, encoded)
        .await
        .map_err(|error| AppError::internal(format!("vault key write: {error}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    tokio::fs::rename(&tmp, target)
        .await
        .map_err(|error| AppError::internal(format!("vault key rename: {error}")))?;
    Ok(keys.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sync_validates_and_writes_keyring() {
        use axum::{routing::get, Router};

        async fn secret() -> axum::Json<Value> {
            serde_json::json!({
                "data": {
                    "data": {
                        "k1": base64::engine::general_purpose::STANDARD.encode([1u8; 32])
                    }
                }
            })
            .into()
        }

        let app = Router::new().route("/v1/secret/data/export", get(secret));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let dir = std::env::temp_dir().join(format!("tsp-vault-{}", uuid::Uuid::new_v4()));
        let path = dir.join("keys.json");
        let count = sync_once(
            &format!("http://{addr}"),
            "token",
            "secret/data/export",
            None,
            &path,
        )
        .await
        .expect("sync");
        assert_eq!(count, 1);
        let written: Value =
            serde_json::from_slice(&tokio::fs::read(&path).await.expect("read")).expect("json");
        assert!(written.get("k1").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn sync_rejects_empty_secret() {
        use axum::{routing::get, Router};

        async fn secret() -> axum::Json<Value> {
            serde_json::json!({"data": {"data": {"bad": "not-base64"}}}).into()
        }
        let app = Router::new().route("/v1/secret/data/export", get(secret));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let path = std::env::temp_dir().join(format!("tsp-vault-{}", uuid::Uuid::new_v4()));
        assert!(sync_once(
            &format!("http://{addr}"),
            "token",
            "secret/data/export",
            None,
            &path
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn approle_login_and_secret_fetch() {
        use axum::{
            http::HeaderMap,
            routing::{get, post},
            Json, Router,
        };

        async fn login() -> Json<Value> {
            serde_json::json!({"auth": {"client_token": "approle-token"}}).into()
        }
        async fn secret(headers: HeaderMap) -> Json<Value> {
            assert_eq!(
                headers.get("x-vault-token").and_then(|v| v.to_str().ok()),
                Some("approle-token")
            );
            serde_json::json!({
                "data": {"data": {"k1": base64::engine::general_purpose::STANDARD.encode([2u8; 32])}}
            })
            .into()
        }
        let app = Router::new()
            .route("/v1/auth/approle/login", post(login))
            .route("/v1/secret/data/export", get(secret));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let token = login_approle(&format!("http://{addr}"), "role", "secret", None)
            .await
            .expect("login");
        assert_eq!(token, "approle-token");
        let dir = std::env::temp_dir().join(format!("tsp-vault-{}", uuid::Uuid::new_v4()));
        let path = dir.join("keys.json");
        sync_once(
            &format!("http://{addr}"),
            &token,
            "secret/data/export",
            None,
            &path,
        )
        .await
        .expect("sync");
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
