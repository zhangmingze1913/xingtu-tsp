use axum::{
    body::{Body, Bytes},
    extract::{Query, State},
    http::{header, StatusCode},
    response::Response,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    auth::AuthUser,
    error::{AppError, AppResult},
    state::AppState,
};

mod export;
pub use export::*;

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

pub(super) fn scoped_history_org(
    user: &AuthUser,
    requested: Option<&str>,
) -> AppResult<Option<String>> {
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

pub(super) fn csv_row(row: &crate::models::TimeseriesReconciliation) -> String {
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
