use axum::{
    body::{Body, Bytes},
    extract::{Path, Query, State},
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

/// Advertises the honest level of standards support implemented by this build.
/// Consumers must not treat `frame=true` as conformance.
pub async fn standards(State(_state): State<AppState>, _user: AuthUser) -> AppResult<Json<Value>> {
    let (done, partial, missing, na) = crate::standards::catalog::counts();
    Ok(Json(json!({
        "catalog_version": crate::standards::catalog::CATALOG_VERSION,
        "catalog_scope": "core-message-subset",
        "catalog_counts": {"done": done, "partial": partial, "missing": missing, "not_applicable": na},
        "jt_t_809_2019": {
            "envelope": true,
            "crc32": true,
            "business_messages": false,
            "encryption": false,
            "platform_handshake": false,
            "conformance_tested": false,
            "negative_codec_tests": true
        },
        "gb_t_32960_3_2025": {
            "envelope": true,
            "vin": true,
            "business_data_units": false,
            "encryption": false,
            "conformance_tested": false
        },
        "jt_t_808_2019": {
            "subset": true,
            "registration_auth_heartbeat_location": true,
            "registration_full_fields": false,
            "registration_dual_layout": true,
            "registration_plate_no": true,
            "attributes_candidate_fields": true,
            "batch_location": true,
            "subpackage_reassembly": true,
            "subpackage_limits": true,
            "text_downlink_0x8300": true,
            "vehicle_control_0x8500": true,
            "parameter_messages": true,
            "parameter_version_history": true,
            "parameter_version_diff": true,
            "parameter_retry": true,
            "downlink_timeout_detection": true,
            "parameter_auto_retry": true,
            "query_all_parameters_0x8104": true,
            "query_attributes_0x8107": true,
            "temp_tracking_0x8202": true,
            "temp_tracking_state": true,
            "link_test_state_0x8204": true,
            "alarm_ack_0x8203": true,
            "alarm_ack_candidate": true,
            "cross_batch_retransmission_dedupe": true,
            "concurrent_batch_idempotency_claims": true,
            "location_extra_items": true,
            "location_extra_semantics": true,
            "location_extra_strict_tlv": true,
            "location_range_validation": true,
            "alarm_id_extra": true,
            "overspeed_area_extra": true,
            "area_event_extra": true,
            "route_time_extra": true,
            "extended_signal_extra": true,
            "batch_chronological_sort": true,
            "single_session_local": true,
            "single_session_cross_node": true,
            "batch_limits": true,
            "batch_rate_limit": true,
            "parameter_response_persistence": true,
            "attributes_response_persistence": true,
            "encryption": false,
            "full_message_catalog": false,
            "conformance_tested": false
        }
    })))
}

#[derive(Debug, Deserialize)]
pub struct ReconcileQuery {
    pub vehicle_id: String,
    pub from: Option<String>,
    pub to: Option<String>,
}

/// Compare PostgreSQL and TDengine counts over the same vehicle/time range.
pub async fn timeseries_reconcile(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<ReconcileQuery>,
) -> AppResult<Json<Value>> {
    user.require_admin()?;
    let tdengine = state.tdengine.as_ref().ok_or_else(|| {
        AppError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "tdengine_unavailable",
            "TDengine is not enabled",
        )
    })?;
    let from = query
        .from
        .unwrap_or_else(|| crate::util::iso_from_now_minutes(-60));
    let to = query.to.unwrap_or_else(crate::util::now_iso);
    let pg = state
        .db
        .telemetry_range_stats(&query.vehicle_id, &from, &to)
        .await?;
    let td = tdengine.range_stats(&query.vehicle_id, &from, &to).await?;
    let delta = pg.count - td.count;
    crate::tdengine::RECONCILE_CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if delta != 0 {
        crate::tdengine::RECONCILE_MISMATCHES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    if delta != 0 {
        crate::tdengine::notify_reconcile_mismatch(
            &query.vehicle_id,
            &from,
            &to,
            pg.count,
            td.count,
            delta,
        )
        .await;
    }
    let org_name = state
        .db
        .vehicle_org_name(&query.vehicle_id)
        .await?
        .unwrap_or_default();
    let id = state
        .db
        .record_timeseries_reconciliation(
            &query.vehicle_id,
            &org_name,
            &from,
            &to,
            pg.count,
            td.count,
            delta,
        )
        .await?;
    Ok(Json(json!({
        "id": id,
        "vehicle_id": query.vehicle_id,
        "from": from,
        "to": to,
        "postgres": pg,
        "tdengine": td,
        "count_delta": delta,
        "consistent": delta == 0,
        "note": "count-based comparison; PG remains authoritative",
    })))
}

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

fn parse_history_cursor(raw: Option<&str>) -> AppResult<Option<(&str, &str)>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let (checked_at, id) = raw
        .split_once('|')
        .ok_or_else(|| AppError::bad_request("cursor 必须是 checked_at|id"))?;
    if checked_at.is_empty() || id.is_empty() {
        return Err(AppError::bad_request("cursor 的 checked_at/id 不能为空"));
    }
    Ok(Some((checked_at, id)))
}

fn scoped_history_org(user: &AuthUser, requested: Option<&str>) -> AppResult<Option<String>> {
    if user.is_admin() {
        return Ok(requested.map(str::to_string));
    }
    if user.org_name.is_empty() {
        return Err(AppError::forbidden("账号未绑定组织，无法查询对账历史"));
    }
    Ok(Some(user.org_name.clone()))
}

#[derive(Debug, Deserialize)]
pub struct ReconcileHistoryQuery {
    pub limit: Option<i64>,
    pub vehicle_id: Option<String>,
    pub org_name: Option<String>,
    pub consistent: Option<bool>,
    pub cursor: Option<String>,
    pub footer: Option<bool>,
}

pub async fn timeseries_reconciliations(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<ReconcileHistoryQuery>,
) -> AppResult<Json<Value>> {
    let org_name = scoped_history_org(&user, query.org_name.as_deref())?;
    let before = parse_history_cursor(query.cursor.as_deref())?;
    let rows = state
        .db
        .list_timeseries_reconciliations_page(
            query.limit.unwrap_or(50).clamp(1, 200),
            before,
            query.vehicle_id.as_deref(),
            org_name.as_deref(),
            query.consistent,
        )
        .await?;
    let next_cursor = rows
        .last()
        .map(|row| format!("{}|{}", row.checked_at, row.id));
    Ok(Json(json!({"items": rows, "next_cursor": next_cursor})))
}

pub async fn timeseries_spool(
    State(_state): State<AppState>,
    user: AuthUser,
) -> AppResult<Json<Value>> {
    user.require_admin()?;
    let Some(spool) = crate::tdengine_spool::global() else {
        return Err(AppError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "spool_disabled",
            "TDengine spool is not configured",
        ));
    };
    let (files, bytes) = spool
        .usage()
        .await
        .map_err(|error| AppError::internal(format!("TDengine spool usage failed: {error}")))?;
    Ok(Json(json!({"files": files, "bytes": bytes})))
}

pub async fn timeseries_spool_replay(
    State(state): State<AppState>,
    user: AuthUser,
) -> AppResult<Json<Value>> {
    user.require_admin()?;
    let tdengine = state.tdengine.as_ref().ok_or_else(|| {
        AppError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "tdengine_unavailable",
            "TDengine is not enabled",
        )
    })?;
    let replayed = tdengine.replay_spool().await;
    Ok(Json(json!({"replayed": replayed})))
}

pub async fn timeseries_reconciliations_export(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<ReconcileHistoryQuery>,
) -> AppResult<Response> {
    let org_name = scoped_history_org(&user, query.org_name.as_deref())?;
    let max_rows = query.limit.unwrap_or(2000).clamp(1, 5000) as usize;
    let footer = query.footer.unwrap_or(false);
    let cursor = parse_history_cursor(query.cursor.as_deref())?;
    let exported = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let page_size = 500i64;
    let db = state.db.clone();
    let vehicle = query.vehicle_id.clone();
    let consistent = query.consistent;
    let state = (
        db,
        vehicle,
        org_name,
        consistent,
        cursor.map(|(checked_at, id)| (checked_at.to_string(), id.to_string())),
        0usize,
    );
    let rows = futures_util::stream::unfold(
        state,
        move |(db, vehicle, org, consistent, cursor, sent)| async move {
            if sent >= max_rows {
                return None;
            }
            let remaining = (max_rows - sent).min(page_size as usize) as i64;
            let before = cursor.as_ref().map(|(ts, id)| (ts.as_str(), id.as_str()));
            match db
                .list_timeseries_reconciliations_page(
                    remaining,
                    before,
                    vehicle.as_deref(),
                    org.as_deref(),
                    consistent,
                )
                .await
            {
                Ok(rows) if rows.is_empty() => None,
                Ok(rows) => {
                    let mut chunk = String::new();
                    for row in &rows {
                        chunk.push_str(&csv_row(row));
                    }
                    let last = rows.last().expect("non-empty");
                    let next = Some((last.checked_at.clone(), last.id.clone()));
                    let sent = sent + rows.len();
                    Some((
                        Ok::<Bytes, std::io::Error>(Bytes::from(chunk)),
                        (db, vehicle, org, consistent, next, sent),
                    ))
                }
                Err(error) => {
                    tracing::warn!(%error, "reconciliation CSV page failed");
                    Some((
                        Err(std::io::Error::other(error.to_string())),
                        (db, vehicle, org, consistent, None, max_rows),
                    ))
                }
            }
        },
    );
    let header = futures_util::stream::once(async {
        Ok::<Bytes, std::io::Error>(Bytes::from(
            "id,vehicle_id,org_name,from_ts,to_ts,pg_count,td_count,count_delta,consistent,checked_at\n",
        ))
    });
    let footer = futures_util::stream::once(async move {
        if footer {
            Ok::<Bytes, std::io::Error>(Bytes::from(csv_footer(
                exported.load(std::sync::atomic::Ordering::Relaxed),
            )))
        } else {
            Ok(Bytes::new())
        }
    });
    let stream =
        futures_util::StreamExt::chain(futures_util::StreamExt::chain(header, rows), footer);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"timeseries-reconciliations.csv\"",
        )
        .body(Body::from_stream(stream))
        .map_err(|error| AppError::internal(format!("CSV response: {error}")))
}

pub async fn timeseries_reconciliations_export_page(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<ReconcileHistoryQuery>,
) -> AppResult<Response> {
    let org_name = scoped_history_org(&user, query.org_name.as_deref())?;
    let before = parse_history_cursor(query.cursor.as_deref())?;
    let limit = query.limit.unwrap_or(500).clamp(1, 500);
    let rows = state
        .db
        .list_timeseries_reconciliations_page(
            limit,
            before,
            query.vehicle_id.as_deref(),
            org_name.as_deref(),
            query.consistent,
        )
        .await?;
    let mut csv = String::from(
        "id,vehicle_id,org_name,from_ts,to_ts,pg_count,td_count,count_delta,consistent,checked_at\n",
    );
    for row in &rows {
        csv.push_str(&csv_row(row));
    }
    let next_cursor = if rows.len() as i64 == limit {
        rows.last()
            .map(|row| format!("{}|{}", row.checked_at, row.id))
    } else {
        None
    };
    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"timeseries-reconciliations-page.csv\"",
        )
        .header("X-Row-Count", rows.len().to_string())
        .body(Body::from(csv))
        .map_err(|error| AppError::internal(format!("CSV page response: {error}")))?;
    if let Some(cursor) = next_cursor {
        response.headers_mut().insert(
            "X-Next-Cursor",
            axum::http::HeaderValue::from_str(&cursor)
                .map_err(|_| AppError::internal("invalid next cursor"))?,
        );
    }
    Ok(response)
}

fn csv_footer(rows: usize) -> String {
    format!("#END,{rows}\n")
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
fn reconciliations_csv_lines(rows: &[crate::models::TimeseriesReconciliation]) -> Vec<String> {
    let mut lines = Vec::with_capacity(rows.len() + 1);
    lines.push(
        "id,vehicle_id,org_name,from_ts,to_ts,pg_count,td_count,count_delta,consistent,checked_at\n"
            .to_string(),
    );
    for row in rows {
        lines.push(csv_row(row));
    }
    lines
}

fn csv_row(row: &crate::models::TimeseriesReconciliation) -> String {
    format!(
        "{},{},{},{},{},{},{},{},{},{}\n",
        csv_field(&row.id),
        csv_field(&row.vehicle_id),
        csv_field(&row.org_name),
        csv_field(&row.from_ts),
        csv_field(&row.to_ts),
        row.pg_count,
        row.td_count,
        row.count_delta,
        row.consistent,
        csv_field(&row.checked_at)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconciliation_csv_has_header_and_rows() {
        let row = crate::models::TimeseriesReconciliation {
            id: "id-1".to_string(),
            vehicle_id: "veh-1".to_string(),
            org_name: "Org-1".to_string(),
            from_ts: "a".to_string(),
            to_ts: "b".to_string(),
            pg_count: 10,
            td_count: 8,
            count_delta: 2,
            consistent: false,
            checked_at: "2026-11-24T00:00:00Z".to_string(),
        };
        let csv = reconciliations_csv_lines(&[row]).concat();
        assert!(csv.starts_with("id,vehicle_id"));
        assert!(csv.contains("id-1,veh-1,Org-1,a,b,10,8,2,false"));
    }

    #[test]
    fn history_org_scope_forces_non_admin_org() {
        let admin = AuthUser {
            id: "1".to_string(),
            username: "admin".to_string(),
            role: "admin".to_string(),
            org_name: "".to_string(),
            jti: "j".to_string(),
        };
        assert_eq!(
            scoped_history_org(&admin, Some("Other"))
                .unwrap()
                .as_deref(),
            Some("Other")
        );
        let operator = AuthUser {
            id: "2".to_string(),
            username: "op".to_string(),
            role: "operator".to_string(),
            org_name: "Org-A".to_string(),
            jti: "j".to_string(),
        };
        assert_eq!(
            scoped_history_org(&operator, Some("Other"))
                .unwrap()
                .as_deref(),
            Some("Org-A")
        );
        let unbound = AuthUser {
            id: "3".to_string(),
            username: "viewer".to_string(),
            role: "viewer".to_string(),
            org_name: "".to_string(),
            jti: "j".to_string(),
        };
        assert!(scoped_history_org(&unbound, None).is_err());
    }

    #[test]
    fn cursor_parser_rejects_malformed_values() {
        assert!(parse_history_cursor(None).unwrap().is_none());
        assert_eq!(
            parse_history_cursor(Some("2026-11-24T00:00:00Z|abc"))
                .unwrap()
                .unwrap(),
            ("2026-11-24T00:00:00Z", "abc")
        );
        assert!(parse_history_cursor(Some("missing")).is_err());
        assert!(parse_history_cursor(Some("|abc")).is_err());
        assert!(parse_history_cursor(Some("ts|")).is_err());
    }

    #[test]
    fn csv_footer_reports_row_count() {
        assert_eq!(csv_footer(42), "#END,42\n");
    }
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

async fn notify_export_failure(job_id: &str, error: &str) {
    let Ok(url) = std::env::var("TSP_RECONCILE_ALERT_WEBHOOK") else {
        return;
    };
    if url.trim().is_empty() {
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
