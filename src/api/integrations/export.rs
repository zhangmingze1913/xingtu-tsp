use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::Response,
    Json,
};
use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;

use crate::{
    auth::AuthUser,
    error::{AppError, AppResult},
    state::AppState,
};

use super::{csv_row, scoped_history_org};

fn export_token_secret() -> Option<String> {
    for name in ["TSP_EXPORT_TOKEN_SECRET", "TSP_JWT_SECRET"] {
        if let Ok(value) = std::env::var(name) {
            if !value.trim().is_empty() {
                return Some(value);
            }
        }
    }
    None
}

fn export_token_ttl() -> i64 {
    std::env::var("TSP_EXPORT_TOKEN_TTL_SECS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .map(|value| value.clamp(30, 3600))
        .unwrap_or(600)
}

fn hmac_hex(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC key");
    mac.update(body);
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn issue_export_token_sync(job_id: &str, jti: &str) -> Option<(String, i64)> {
    let secret = export_token_secret()?;
    let ttl = export_token_ttl();
    let expires = chrono::Utc::now().timestamp() + ttl;
    let payload = format!("{job_id}.{expires}.{jti}");
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.as_bytes());
    let signature = hmac_hex(&secret, encoded.as_bytes());
    Some((format!("{encoded}.{signature}"), ttl))
}

async fn issue_export_token(state: &AppState, job_id: &str) -> Option<(String, i64)> {
    let jti = uuid::Uuid::new_v4().simple().to_string();
    let (token, ttl) = issue_export_token_sync(job_id, &jti)?;
    let expires_at = (chrono::Utc::now() + chrono::Duration::seconds(ttl))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    if state
        .db
        .create_export_download_token(&jti, job_id, &expires_at)
        .await
        .is_err()
    {
        return None;
    }
    Some((token, ttl))
}

fn verify_export_token(token: &str) -> Option<(String, String)> {
    let secret = export_token_secret()?;
    let (encoded, signature) = token.rsplit_once('.')?;
    let expected = hmac_hex(&secret, encoded.as_bytes());
    if expected != signature {
        return None;
    }
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .ok()?;
    let payload = String::from_utf8(decoded).ok()?;
    let (job_id, rest) = payload.split_once('.')?;
    let (expires, jti) = rest.rsplit_once('.')?;
    if expires.parse::<i64>().ok()? < chrono::Utc::now().timestamp() {
        return None;
    }
    if jti.is_empty() {
        return None;
    }
    Some((job_id.to_string(), jti.to_string()))
}

#[derive(Debug, Deserialize)]
pub struct ExportJobRequest {
    pub vehicle_id: Option<String>,
    pub org_name: Option<String>,
    pub consistent: Option<bool>,
    pub max_rows: Option<i64>,
}

pub async fn start_reconciliation_export_job(
    State(state): State<AppState>,
    user: AuthUser,
    Json(request): Json<ExportJobRequest>,
) -> AppResult<Json<Value>> {
    let org_name = scoped_history_org(&user, request.org_name.as_deref())?;
    let max_rows = request.max_rows.unwrap_or(5000).clamp(1, 5000);
    let id = state
        .db
        .create_reconciliation_export_job(
            org_name.as_deref().unwrap_or(""),
            request.vehicle_id.as_deref(),
            request.consistent,
            max_rows,
        )
        .await?;
    crate::tdengine::EXPORT_JOBS_STARTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let worker_state = state.clone();
    let worker_id = id.clone();
    let worker_org = org_name.clone();
    let worker_vehicle = request.vehicle_id.clone();
    let worker_consistent = request.consistent;
    tokio::spawn(async move {
        run_reconciliation_export(
            worker_state,
            worker_id,
            worker_org,
            worker_vehicle,
            worker_consistent,
            max_rows,
        )
        .await;
    });
    Ok(Json(json!({"id": id, "status": "running"})))
}

async fn run_reconciliation_export(
    state: AppState,
    id: String,
    org_name: Option<String>,
    vehicle_id: Option<String>,
    consistent: Option<bool>,
    max_rows: i64,
) {
    if let Err(error) =
        run_reconciliation_export_inner(&state, &id, org_name, vehicle_id, consistent, max_rows)
            .await
    {
        crate::tdengine::EXPORT_JOBS_FAILED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        tracing::warn!(%error, job = %id, "reconciliation export failed");
        let message = error.to_string();
        let _ = state
            .db
            .update_reconciliation_export_job(&id, "failed", 0, None, Some(&message))
            .await;
        notify_export_failure(&id, &message).await;
    }
}

async fn run_reconciliation_export_inner(
    state: &AppState,
    id: &str,
    org_name: Option<String>,
    vehicle_id: Option<String>,
    consistent: Option<bool>,
    max_rows: i64,
) -> AppResult<()> {
    let dir = std::env::var("TSP_RECONCILE_EXPORT_DIR")
        .unwrap_or_else(|_| "data/reconcile-exports".to_string());
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|error| AppError::internal(format!("export dir: {error}")))?;
    let job = state
        .db
        .get_reconciliation_export_job(id)
        .await?
        .ok_or_else(|| AppError::not_found("导出任务不存在"))?;
    if job.status == "cancelled" {
        return Ok(());
    }
    let lease_owner = uuid::Uuid::new_v4().simple().to_string();
    let now = chrono::Utc::now();
    let now_text = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let lease_until =
        (now + chrono::Duration::seconds(120)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    if !state
        .db
        .try_acquire_export_lease(id, &lease_owner, &now_text, &lease_until)
        .await?
    {
        return Ok(());
    }
    if crate::export_crypto::encryption_required() && crate::export_crypto::key_from_env().is_none()
    {
        return Err(AppError::internal(
            "已启用强制加密，但未配置有效的 TSP_EXPORT_ENCRYPTION_KEY",
        ));
    }
    let final_path = format!("{dir}/{id}.csv");
    let encrypted_path = format!("{final_path}.enc");
    let _ = tokio::fs::remove_file(&encrypted_path).await;
    let mut written = job.rows_written.max(0);
    let mut cursor = match (job.cursor_ts.clone(), job.cursor_id.clone()) {
        (Some(ts), Some(cursor_id)) => Some((ts, cursor_id)),
        _ => None,
    };
    let resume = written > 0
        && tokio::fs::metadata(&final_path)
            .await
            .map(|meta| meta.len() > 0)
            .unwrap_or(false);
    use tokio::io::AsyncWriteExt;
    let mut file = if resume {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(&final_path)
            .await
            .map_err(|error| AppError::internal(format!("export open: {error}")))?
    } else {
        written = 0;
        cursor = None;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&final_path)
            .await
            .map_err(|error| AppError::internal(format!("export create: {error}")))?;
        file.write_all(
            b"id,vehicle_id,org_name,from_ts,to_ts,pg_count,td_count,count_delta,consistent,checked_at\n",
        )
        .await
        .map_err(|error| AppError::internal(format!("export header: {error}")))?;
        file
    };
    loop {
        if let Ok(Some(current)) = state.db.get_reconciliation_export_job(id).await {
            if current.status == "cancelled" {
                drop(file);
                let _ = tokio::fs::remove_file(&final_path).await;
                let _ = state.db.release_export_lease(id, &lease_owner).await;
                return Ok(());
            }
        }
        let remaining = (max_rows - written).clamp(1, 500);
        let page_timeout = std::env::var("TSP_RECONCILE_EXPORT_PAGE_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(|value| value.clamp(5, 100))
            .unwrap_or(60);
        let page = tokio::time::timeout(
            std::time::Duration::from_secs(page_timeout),
            state.db.list_timeseries_reconciliations_page(
                remaining,
                cursor
                    .as_ref()
                    .map(|(ts, row_id)| (ts.as_str(), row_id.as_str())),
                vehicle_id.as_deref(),
                org_name.as_deref(),
                consistent,
            ),
        )
        .await
        .map_err(|_| AppError::internal("export page timed out"))??;
        if page.is_empty() {
            break;
        }
        let mut chunk = String::new();
        for row in &page {
            chunk.push_str(&csv_row(row));
        }
        file.write_all(chunk.as_bytes())
            .await
            .map_err(|error| AppError::internal(format!("export write: {error}")))?;
        written += page.len() as i64;
        let last = page.last().expect("non-empty");
        cursor = Some((last.checked_at.clone(), last.id.clone()));
        state
            .db
            .update_reconciliation_export_progress(
                id,
                written,
                cursor.as_ref().map(|(ts, _)| ts.as_str()),
                cursor.as_ref().map(|(_, row_id)| row_id.as_str()),
            )
            .await?;
        let renew_until = (chrono::Utc::now() + chrono::Duration::seconds(120))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        if !state
            .db
            .renew_export_lease(id, &lease_owner, &renew_until)
            .await?
        {
            tracing::warn!(job = %id, "export lease lost; stopping worker");
            return Ok(());
        }
        if page.len() < remaining as usize || written >= max_rows {
            break;
        }
    }
    file.flush()
        .await
        .map_err(|error| AppError::internal(format!("export flush: {error}")))?;
    if let Ok(Some(current)) = state.db.get_reconciliation_export_job(id).await {
        if current.status == "cancelled" {
            drop(file);
            let _ = tokio::fs::remove_file(&final_path).await;
            return Ok(());
        }
    }
    state
        .db
        .update_reconciliation_export_progress(id, written, None, None)
        .await?;
    let stored_path = if crate::export_crypto::encryption_configured() {
        let plaintext = tokio::fs::read(&final_path)
            .await
            .map_err(|error| AppError::internal(format!("export read for encryption: {error}")))?;
        let encrypted = crate::export_crypto::encrypt_with_env(&plaintext)?;
        tokio::fs::write(&encrypted_path, encrypted)
            .await
            .map_err(|error| AppError::internal(format!("export encrypt write: {error}")))?;
        let _ = tokio::fs::remove_file(&final_path).await;
        encrypted_path
    } else {
        final_path
    };
    state
        .db
        .update_reconciliation_export_job(id, "completed", written, Some(&stored_path), None)
        .await?;
    let _ = state.db.release_export_lease(id, &lease_owner).await;
    crate::tdengine::EXPORT_JOBS_COMPLETED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

fn export_failure_payload(format: &str, job_id: &str, error: &str) -> Value {
    let checked_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    if format.eq_ignore_ascii_case("alertmanager") {
        json!([{
            "labels": {
                "alertname": "ReconciliationExportFailed",
                "severity": "warning",
                "service": "xingtu-tsp",
                "job_id": job_id,
            },
            "annotations": {
                "summary": "Reconciliation export job failed",
                "description": error,
            },
            "startsAt": checked_at,
        }])
    } else {
        json!({
            "type": "reconciliation_export_failed",
            "job_id": job_id,
            "error": error,
            "checked_at": checked_at,
        })
    }
}

static EXPORT_ALERT_LAST: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, i64>>,
> = std::sync::OnceLock::new();

fn export_alert_allowed(job_id: &str, now: i64, cooldown: i64) -> bool {
    let map =
        EXPORT_ALERT_LAST.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let Ok(mut map) = map.lock() else {
        return true;
    };
    if map.len() > 1024 {
        map.retain(|_, at| now - *at < cooldown);
    }
    if let Some(previous) = map.get(job_id) {
        if now - *previous < cooldown {
            return false;
        }
    }
    map.insert(job_id.to_string(), now);
    true
}

async fn notify_export_failure(job_id: &str, error: &str) {
    let Ok(url) = std::env::var("TSP_RECONCILE_ALERT_WEBHOOK") else {
        return;
    };
    if url.trim().is_empty() {
        return;
    }
    let cooldown = std::env::var("TSP_EXPORT_ALERT_COOLDOWN_SECS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(300)
        .max(1);
    if !export_alert_allowed(job_id, chrono::Utc::now().timestamp(), cooldown) {
        crate::tdengine::EXPORT_ALERTS_SKIPPED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return;
    }
    let format =
        std::env::var("TSP_RECONCILE_ALERT_FORMAT").unwrap_or_else(|_| "generic".to_string());
    let payload = export_failure_payload(&format, job_id, error);
    match crate::tdengine::send_webhook(url.trim(), &payload).await {
        Ok(()) => {
            crate::tdengine::EXPORT_ALERTS_SENT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Err(send_error) => {
            crate::tdengine::EXPORT_ALERTS_FAILED
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            tracing::warn!(%send_error, job = %job_id, "export failure alert failed");
        }
    }
}

async fn read_export_csv(id: &str) -> AppResult<Vec<u8>> {
    let dir = std::env::var("TSP_RECONCILE_EXPORT_DIR")
        .unwrap_or_else(|_| "data/reconcile-exports".to_string());
    let encrypted_path = format!("{dir}/{id}.csv.enc");
    if tokio::fs::metadata(&encrypted_path).await.is_ok() {
        let encoded = tokio::fs::read(&encrypted_path)
            .await
            .map_err(|error| AppError::internal(format!("export read: {error}")))?;
        return crate::export_crypto::decrypt_with_env(&encoded);
    }
    tokio::fs::read(format!("{dir}/{id}.csv"))
        .await
        .map_err(|error| AppError::internal(format!("export read: {error}")))
}

pub async fn reconciliation_export_job(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let job = state
        .db
        .get_reconciliation_export_job(&id)
        .await?
        .ok_or_else(|| AppError::not_found("导出任务不存在"))?;
    if !user.is_admin() && job.org_name != user.org_name {
        return Err(AppError::forbidden("无权访问该导出任务"));
    }
    let mut value = serde_json::to_value(&job)
        .map_err(|error| AppError::internal(format!("job serialize: {error}")))?;
    if job.status == "completed" {
        if let Some((token, ttl)) = issue_export_token(&state, &id).await {
            value["download_token"] = json!(token);
            value["download_expires_in"] = json!(ttl);
        }
    }
    Ok(Json(value))
}

pub async fn download_reconciliation_export_job(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Response> {
    let job = state
        .db
        .get_reconciliation_export_job(&id)
        .await?
        .ok_or_else(|| AppError::not_found("导出任务不存在"))?;
    if !user.is_admin() && job.org_name != user.org_name {
        return Err(AppError::forbidden("无权访问该导出任务"));
    }
    if job.status != "completed" {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "export_not_ready",
            "导出任务尚未完成",
        ));
    }
    let bytes = read_export_csv(&id).await?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"reconciliation-export-{id}.csv\""),
        )
        .body(Body::from(bytes))
        .map_err(|error| AppError::internal(format!("export response: {error}")))
}

pub async fn cancel_reconciliation_export_job(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let job = state
        .db
        .get_reconciliation_export_job(&id)
        .await?
        .ok_or_else(|| AppError::not_found("导出任务不存在"))?;
    if !user.is_admin() && job.org_name != user.org_name {
        return Err(AppError::forbidden("无权取消该导出任务"));
    }
    if state.db.cancel_reconciliation_export_job(&id).await? {
        crate::tdengine::EXPORT_JOBS_CANCELLED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    } else {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "export_not_running",
            "导出任务不在运行中",
        ));
    }
    Ok(Json(json!({"id": id, "status": "cancelled"})))
}

pub async fn retry_reconciliation_export_job(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let job = state
        .db
        .get_reconciliation_export_job(&id)
        .await?
        .ok_or_else(|| AppError::not_found("导出任务不存在"))?;
    if !user.is_admin() && job.org_name != user.org_name {
        return Err(AppError::forbidden("无权重试该导出任务"));
    }
    if !state.db.reset_reconciliation_export_job(&id).await? {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "export_not_retryable",
            "仅 failed/cancelled 任务可重试",
        ));
    }
    crate::tdengine::EXPORT_JOBS_RETRIED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::tdengine::EXPORT_JOBS_STARTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let worker_state = state.clone();
    let worker_id = id.clone();
    tokio::spawn(async move {
        run_reconciliation_export(
            worker_state,
            worker_id,
            Some(job.org_name),
            job.vehicle_id,
            job.consistent,
            job.max_rows,
        )
        .await;
    });
    Ok(Json(
        json!({"id": id, "status": "running", "retried": true}),
    ))
}

pub async fn resume_reconciliation_export_job(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let job = state
        .db
        .get_reconciliation_export_job(&id)
        .await?
        .ok_or_else(|| AppError::not_found("导出任务不存在"))?;
    if !user.is_admin() && job.org_name != user.org_name {
        return Err(AppError::forbidden("无权恢复该导出任务"));
    }
    if job.status != "running" {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "export_not_resumable",
            "仅 running 任务可恢复",
        ));
    }
    let stale_secs = std::env::var("TSP_RECONCILE_EXPORT_STALE_SECS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(60)
        .max(10);
    let updated = chrono::DateTime::parse_from_rfc3339(&job.updated_at)
        .map_err(|_| AppError::internal("导出任务时间戳无效"))?
        .with_timezone(&chrono::Utc);
    if (chrono::Utc::now() - updated).num_seconds() < stale_secs {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "export_still_active",
            "任务 recently active，暂不能恢复",
        ));
    }
    crate::tdengine::EXPORT_JOBS_RESUMED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let worker_state = state.clone();
    let worker_id = id.clone();
    tokio::spawn(async move {
        run_reconciliation_export(
            worker_state,
            worker_id,
            Some(job.org_name),
            job.vehicle_id,
            job.consistent,
            job.max_rows,
        )
        .await;
    });
    Ok(Json(
        json!({"id": id, "status": "running", "resumed": true}),
    ))
}

pub(crate) async fn recover_export_job(state: AppState, id: String) {
    let Ok(Some(job)) = state.db.get_reconciliation_export_job(&id).await else {
        return;
    };
    if job.status != "running" {
        return;
    }
    crate::tdengine::EXPORT_JOBS_RECOVERED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    tokio::spawn(async move {
        run_reconciliation_export(
            state,
            id,
            Some(job.org_name),
            job.vehicle_id,
            job.consistent,
            job.max_rows,
        )
        .await;
    });
}

pub async fn download_reconciliation_export_by_token(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> AppResult<Response> {
    let (id, jti) = verify_export_token(&token)
        .ok_or_else(|| AppError::unauthorized("下载令牌无效或已过期"))?;
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    if !state.db.consume_export_download_token(&jti, &now).await? {
        return Err(AppError::unauthorized("下载令牌已使用或已过期"));
    }
    let job = state
        .db
        .get_reconciliation_export_job(&id)
        .await?
        .ok_or_else(|| AppError::not_found("导出任务不存在"))?;
    if job.status != "completed" {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "export_not_ready",
            "导出任务尚未完成",
        ));
    }
    let bytes = read_export_csv(&id).await?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"reconciliation-export-{id}.csv\""),
        )
        .body(Body::from(bytes))
        .map_err(|error| AppError::internal(format!("export response: {error}")))
}

#[cfg(test)]
mod token_tests {
    use super::*;

    #[test]
    fn export_download_token_roundtrip() {
        std::env::set_var("TSP_EXPORT_TOKEN_SECRET", "test-export-secret");
        std::env::set_var("TSP_EXPORT_TOKEN_TTL_SECS", "60");
        let (token, ttl) = issue_export_token_sync("job-1", "jti-1").expect("token");
        assert_eq!(ttl, 60);
        assert_eq!(
            verify_export_token(&token).map(|(job, _)| job).as_deref(),
            Some("job-1")
        );
        let mut tampered = token.clone();
        tampered.push('0');
        assert!(verify_export_token(&tampered).is_none());
    }

    #[test]
    fn export_failure_alert_cooldown_is_per_job() {
        assert!(export_alert_allowed("dedupe-a", 100, 300));
        assert!(!export_alert_allowed("dedupe-a", 101, 300));
        assert!(export_alert_allowed("dedupe-b", 101, 300));
        assert!(export_alert_allowed("dedupe-a", 401, 300));
    }

    #[test]
    fn export_failure_payload_supports_formats() {
        let generic = export_failure_payload("generic", "job-1", "boom");
        assert_eq!(generic["type"], "reconciliation_export_failed");
        assert_eq!(generic["job_id"], "job-1");
        let am = export_failure_payload("alertmanager", "job-1", "boom");
        assert_eq!(
            am.as_array().expect("array")[0]["labels"]["alertname"],
            "ReconciliationExportFailed"
        );
    }
}
