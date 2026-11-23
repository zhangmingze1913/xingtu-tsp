use axum::{extract::Path, extract::State, Json};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    auth::AuthUser,
    error::{AppError, AppResult},
    jt808,
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct ParameterInput {
    pub id: u32,
    pub value_hex: String,
}

#[derive(Debug, Deserialize)]
pub struct ParametersRequest {
    pub parameters: Vec<ParameterInput>,
}

#[derive(Debug, Deserialize)]
pub struct ControlRequest {
    pub action: Option<String>,
    pub flag: Option<u8>,
    #[serde(default)]
    pub parameter_hex: String,
}

#[derive(Debug, Deserialize)]
pub struct QueryParametersRequest {
    pub ids: Vec<u32>,
}

#[derive(Debug, Deserialize)]
pub struct TextCommandRequest {
    pub text: String,
    #[serde(default)]
    pub flag: u8,
}

#[derive(Debug, Deserialize)]
pub struct AlarmAckRequest {
    pub alarm_serial: u16,
    pub alarm_type: u32,
}

#[derive(Debug, Deserialize)]
pub struct TempTrackingRequest {
    pub interval_seconds: u16,
    pub expiry_seconds: u32,
}

async fn ensure_terminal(state: &AppState, phone: &str) -> AppResult<()> {
    if state.db.get_terminal_by_phone(phone).await?.is_none() {
        return Err(AppError::not_found("终端不存在"));
    }
    Ok(())
}

async fn send(state: &AppState, phone: &str, msg_id: u16, body: Vec<u8>) -> AppResult<u16> {
    if let Ok(serial) = jt808::send_frame(phone, msg_id, &body) {
        jt808::track_downlink(phone, msg_id, serial);
        state
            .metrics
            .jt808_downlinks
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return Ok(serial);
    }
    let Some(session) = state.cluster.lookup_session(phone).await else {
        return Err(AppError::conflict("终端未连接任何网关副本"));
    };
    let serial = jt808::next_command_serial();
    let frame = jt808::build_frame(msg_id, phone, serial, &body)
        .map_err(|error| AppError::internal(format!("构建下行帧失败: {error}")))?;
    let payload = json!({
        "phone": phone,
        "serial": serial,
        "command_id": "",
        "frame": base64::engine::general_purpose::STANDARD.encode(frame),
    })
    .to_string();
    if !state
        .cluster
        .publish_gateway(&session.node_id, &payload)
        .await
    {
        return Err(AppError::internal("跨节点下行失败"));
    }
    jt808::track_downlink(phone, msg_id, serial);
    state
        .metrics
        .jt808_downlinks
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(serial)
}

pub async fn set_parameters(
    State(state): State<AppState>,
    user: AuthUser,
    Path(phone): Path<String>,
    Json(payload): Json<ParametersRequest>,
) -> AppResult<Json<Value>> {
    user.require_write()?;
    ensure_terminal(&state, &phone).await?;
    let parameters: Vec<jt808::TerminalParameter> = payload
        .parameters
        .into_iter()
        .map(|item| {
            Ok(jt808::TerminalParameter {
                id: item.id,
                value: decode_hex(&item.value_hex)?,
            })
        })
        .collect::<AppResult<_>>()?;
    let body = jt808::encode_terminal_parameters(&parameters)
        .map_err(|error| AppError::bad_request(format!("参数体错误: {error}")))?;
    let serial = send(&state, &phone, jt808::MSG_SET_PARAMETERS, body).await?;
    let values: Vec<_> = parameters
        .iter()
        .map(|parameter| {
            serde_json::json!({
                "id": parameter.id,
                "value_hex": parameter
                    .value
                    .iter()
                    .map(|byte| format!("{byte:02X}"))
                    .collect::<String>()
            })
        })
        .collect();
    if let Some(terminal) = state.db.get_terminal_by_phone(&phone).await? {
        let json = serde_json::json!(values).to_string();
        if let Err(error) = state.db.save_terminal_parameters(&terminal.id, &json).await {
            tracing::warn!(%error, %phone, "save requested terminal parameters failed");
        }
        if let Err(error) = state
            .db
            .save_terminal_parameter_version(&terminal.id, &json)
            .await
        {
            tracing::warn!(%error, %phone, "save requested parameter version failed");
        }
    }
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "terminal.set_parameters",
            &phone,
            &format!("下发 {} 个参数", parameters.len()),
        )
        .await?;
    Ok(Json(
        json!({"serial": serial, "msg_id": jt808::MSG_SET_PARAMETERS}),
    ))
}

pub async fn control(
    State(state): State<AppState>,
    user: AuthUser,
    Path(phone): Path<String>,
    Json(payload): Json<ControlRequest>,
) -> AppResult<Json<Value>> {
    user.require_admin()?;
    ensure_terminal(&state, &phone).await?;
    let flag = payload
        .flag
        .ok_or_else(|| AppError::bad_request("终端控制必须提供 flag"))?;
    let body = jt808::encode_terminal_control(flag, &decode_hex(&payload.parameter_hex)?);
    let serial = send(&state, &phone, jt808::MSG_TERMINAL_CONTROL, body).await?;
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "terminal.control",
            &phone,
            &format!("终端控制 flag={flag}"),
        )
        .await?;
    Ok(Json(
        json!({"serial": serial, "msg_id": jt808::MSG_TERMINAL_CONTROL}),
    ))
}

pub async fn vehicle_control(
    State(state): State<AppState>,
    user: AuthUser,
    Path(phone): Path<String>,
    Json(payload): Json<ControlRequest>,
) -> AppResult<Json<Value>> {
    user.require_admin()?;
    ensure_terminal(&state, &phone).await?;
    let (flag, parameter) = if let Some(action) = payload.action.as_deref() {
        jt808::vehicle_control_action(action)
            .ok_or_else(|| AppError::bad_request("未知车控动作"))?
    } else {
        let flag = payload
            .flag
            .ok_or_else(|| AppError::bad_request("车控必须提供 action 或 flag"))?;
        (flag, decode_hex(&payload.parameter_hex)?)
    };
    let body = jt808::encode_vehicle_control(flag, &parameter);
    let serial = send(&state, &phone, jt808::MSG_VEHICLE_CONTROL, body).await?;
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "terminal.vehicle_control",
            &phone,
            &format!(
                "车辆控制 action={} flag={flag}",
                payload.action.as_deref().unwrap_or("-")
            ),
        )
        .await?;
    Ok(Json(
        json!({"serial": serial, "msg_id": jt808::MSG_VEHICLE_CONTROL}),
    ))
}

pub async fn downlink_status(
    State(state): State<AppState>,
    user: AuthUser,
    Path((phone, serial)): Path<(String, u16)>,
) -> AppResult<Json<Value>> {
    user.require_write()?;
    ensure_terminal(&state, &phone).await?;
    jt808::downlink_status(&phone, serial)
        .map(|status| Json(serde_json::to_value(status).unwrap_or_else(|_| json!({}))))
        .ok_or_else(|| AppError::not_found("下行记录不存在"))
}

pub async fn query_location(
    State(state): State<AppState>,
    user: AuthUser,
    Path(phone): Path<String>,
) -> AppResult<Json<Value>> {
    user.require_write()?;
    ensure_terminal(&state, &phone).await?;
    let serial = send(&state, &phone, jt808::MSG_LOCATION_QUERY, vec![]).await?;
    Ok(Json(
        json!({"serial": serial, "msg_id": jt808::MSG_LOCATION_QUERY}),
    ))
}

pub async fn query_parameters(
    State(state): State<AppState>,
    user: AuthUser,
    Path(phone): Path<String>,
    Json(payload): Json<QueryParametersRequest>,
) -> AppResult<Json<Value>> {
    user.require_write()?;
    ensure_terminal(&state, &phone).await?;
    let body = jt808::encode_specific_parameter_query(&payload.ids)
        .map_err(|error| AppError::bad_request(format!("参数查询体错误: {error}")))?;
    let serial = send(&state, &phone, jt808::MSG_QUERY_SPECIFIC_PARAMETERS, body).await?;
    Ok(Json(json!({
        "serial": serial,
        "msg_id": jt808::MSG_QUERY_SPECIFIC_PARAMETERS
    })))
}

fn decode_hex(value: &str) -> AppResult<Vec<u8>> {
    let value = value.trim();
    if !value.len().is_multiple_of(2) {
        return Err(AppError::bad_request("hex 长度必须为偶数"));
    }
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16)
                .map_err(|_| AppError::bad_request("非法 hex"))
        })
        .collect()
}

pub async fn query_all_parameters(
    State(state): State<AppState>,
    user: AuthUser,
    Path(phone): Path<String>,
) -> AppResult<Json<Value>> {
    user.require_write()?;
    ensure_terminal(&state, &phone).await?;
    let serial = send(
        &state,
        &phone,
        jt808::MSG_QUERY_PARAMETERS,
        jt808::encode_query_all_parameters(),
    )
    .await?;
    Ok(Json(json!({
        "serial": serial,
        "msg_id": jt808::MSG_QUERY_PARAMETERS
    })))
}

pub async fn query_attributes(
    State(state): State<AppState>,
    user: AuthUser,
    Path(phone): Path<String>,
) -> AppResult<Json<Value>> {
    user.require_write()?;
    ensure_terminal(&state, &phone).await?;
    let serial = send(
        &state,
        &phone,
        jt808::MSG_QUERY_TERMINAL_ATTRIBUTES,
        jt808::encode_query_terminal_attributes(),
    )
    .await?;
    Ok(Json(json!({
        "serial": serial,
        "msg_id": jt808::MSG_QUERY_TERMINAL_ATTRIBUTES
    })))
}

pub async fn text_command(
    State(state): State<AppState>,
    user: AuthUser,
    Path(phone): Path<String>,
    Json(payload): Json<TextCommandRequest>,
) -> AppResult<Json<Value>> {
    user.require_write()?;
    ensure_terminal(&state, &phone).await?;
    if payload.text.trim().is_empty() {
        return Err(AppError::bad_request("text 不能为空"));
    }
    let body = jt808::encode_text_command(payload.flag, &payload.text)
        .map_err(|error| AppError::bad_request(format!("文本消息体错误: {error}")))?;
    let serial = send(&state, &phone, jt808::MSG_TEXT_COMMAND, body).await?;
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "terminal.text_command",
            &phone,
            &format!("下发文本消息 flag={}", payload.flag),
        )
        .await?;
    Ok(Json(json!({
        "serial": serial,
        "msg_id": jt808::MSG_TEXT_COMMAND
    })))
}

pub async fn temp_tracking(
    State(state): State<AppState>,
    user: AuthUser,
    Path(phone): Path<String>,
    Json(payload): Json<TempTrackingRequest>,
) -> AppResult<Json<Value>> {
    user.require_write()?;
    ensure_terminal(&state, &phone).await?;
    if payload.interval_seconds == 0 || payload.expiry_seconds == 0 {
        return Err(AppError::bad_request("interval/expiry 必须大于 0"));
    }
    let body = jt808::encode_temp_tracking(payload.interval_seconds, payload.expiry_seconds);
    let serial = send(&state, &phone, jt808::MSG_TEMP_TRACKING, body).await?;
    jt808::register_temp_tracking(
        &phone,
        serial,
        payload.interval_seconds,
        payload.expiry_seconds,
    );
    Ok(Json(json!({
        "serial": serial,
        "msg_id": jt808::MSG_TEMP_TRACKING
    })))
}

pub async fn get_parameters(
    State(state): State<AppState>,
    user: AuthUser,
    Path(phone): Path<String>,
) -> AppResult<Json<Value>> {
    user.require_write()?;
    let terminal = state
        .db
        .get_terminal_by_phone(&phone)
        .await?
        .ok_or_else(|| AppError::not_found("终端不存在"))?;
    let (parameters, _, updated_at) = state
        .db
        .terminal_profile(&terminal.id)
        .await?
        .ok_or_else(|| AppError::not_found("终端尚未上报参数"))?;
    Ok(Json(json!({
        "parameters": parameters,
        "updated_at": updated_at
    })))
}

pub async fn get_attributes(
    State(state): State<AppState>,
    user: AuthUser,
    Path(phone): Path<String>,
) -> AppResult<Json<Value>> {
    user.require_write()?;
    let terminal = state
        .db
        .get_terminal_by_phone(&phone)
        .await?
        .ok_or_else(|| AppError::not_found("终端不存在"))?;
    let (_, attributes, updated_at) = state
        .db
        .terminal_profile(&terminal.id)
        .await?
        .ok_or_else(|| AppError::not_found("终端尚未上报属性"))?;
    let registration = state.db.terminal_registration(&terminal.id).await?;
    Ok(Json(json!({
        "attributes": attributes,
        "registration": registration,
        "updated_at": updated_at
    })))
}

pub async fn get_temp_tracking(
    State(state): State<AppState>,
    user: AuthUser,
    Path(phone): Path<String>,
) -> AppResult<Json<Value>> {
    user.require_write()?;
    ensure_terminal(&state, &phone).await?;
    let status = jt808::temp_tracking_status(&phone)
        .ok_or_else(|| AppError::not_found("没有有效的临时跟踪"))?;
    Ok(Json(
        serde_json::to_value(status).unwrap_or_else(|_| json!({})),
    ))
}

pub async fn link_test(
    State(state): State<AppState>,
    user: AuthUser,
    Path(phone): Path<String>,
) -> AppResult<Json<Value>> {
    user.require_write()?;
    ensure_terminal(&state, &phone).await?;
    let serial = send(&state, &phone, jt808::MSG_LINK_TEST, vec![]).await?;
    jt808::register_link_test(&phone, serial);
    Ok(Json(json!({
        "serial": serial,
        "msg_id": jt808::MSG_LINK_TEST
    })))
}

pub async fn get_link_test(
    State(state): State<AppState>,
    user: AuthUser,
    Path(phone): Path<String>,
) -> AppResult<Json<Value>> {
    user.require_write()?;
    ensure_terminal(&state, &phone).await?;
    let status =
        jt808::link_test_status(&phone).ok_or_else(|| AppError::not_found("没有链路检测记录"))?;
    Ok(Json(
        serde_json::to_value(status).unwrap_or_else(|_| json!({})),
    ))
}

pub async fn alarm_ack(
    State(state): State<AppState>,
    user: AuthUser,
    Path(phone): Path<String>,
    Json(payload): Json<AlarmAckRequest>,
) -> AppResult<Json<Value>> {
    user.require_write()?;
    ensure_terminal(&state, &phone).await?;
    let body = jt808::encode_alarm_ack(payload.alarm_serial, payload.alarm_type);
    let serial = send(&state, &phone, jt808::MSG_ALARM_ACK, body).await?;
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "terminal.alarm_ack",
            &phone,
            &format!(
                "告警确认 serial={} type={:#010x}",
                payload.alarm_serial, payload.alarm_type
            ),
        )
        .await?;
    Ok(Json(json!({
        "serial": serial,
        "msg_id": jt808::MSG_ALARM_ACK
    })))
}

pub async fn parameter_history(
    State(state): State<AppState>,
    user: AuthUser,
    Path(phone): Path<String>,
) -> AppResult<Json<Value>> {
    user.require_write()?;
    let terminal = state
        .db
        .get_terminal_by_phone(&phone)
        .await?
        .ok_or_else(|| AppError::not_found("终端不存在"))?;
    let rows = state
        .db
        .terminal_parameter_versions(&terminal.id, 20)
        .await?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|(version, parameters, created_at)| {
            json!({
                "version": version,
                "parameters": parameters,
                "created_at": created_at
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

pub async fn parameter_diff(
    State(state): State<AppState>,
    user: AuthUser,
    Path(phone): Path<String>,
) -> AppResult<Json<Value>> {
    user.require_write()?;
    let terminal = state
        .db
        .get_terminal_by_phone(&phone)
        .await?
        .ok_or_else(|| AppError::not_found("终端不存在"))?;
    let rows = state
        .db
        .terminal_parameter_versions(&terminal.id, 2)
        .await?;
    if rows.len() < 2 {
        return Err(AppError::not_found("至少需要两个参数版本"));
    }
    let (to_version, to_json, _) = &rows[0];
    let (from_version, from_json, _) = &rows[1];
    Ok(Json(diff_parameter_sets(
        *from_version,
        *to_version,
        from_json,
        to_json,
    )))
}

fn diff_parameter_sets(from_version: i64, to_version: i64, old: &str, new: &str) -> Value {
    let parse = |raw: &str| -> std::collections::BTreeMap<u32, String> {
        serde_json::from_str::<Vec<Value>>(raw)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| {
                Some((
                    u32::try_from(item.get("id")?.as_u64()?).ok()?,
                    item.get("value_hex")?.as_str()?.to_string(),
                ))
            })
            .collect()
    };
    let before = parse(old);
    let after = parse(new);
    let mut changes = Vec::new();
    for (id, value) in &after {
        match before.get(id) {
            Some(previous) if previous == value => {}
            Some(previous) => changes.push(json!({
                "id": id, "change": "changed",
                "from": previous, "to": value
            })),
            None => changes.push(json!({
                "id": id, "change": "added", "to": value
            })),
        }
    }
    for (id, value) in &before {
        if !after.contains_key(id) {
            changes.push(json!({
                "id": id, "change": "removed", "from": value
            }));
        }
    }
    json!({
        "from_version": from_version,
        "to_version": to_version,
        "changes": changes
    })
}

pub(crate) async fn retry_latest_parameters(
    state: &AppState,
    phone: &str,
) -> AppResult<(u16, i64)> {
    let terminal = state
        .db
        .get_terminal_by_phone(phone)
        .await?
        .ok_or_else(|| AppError::not_found("终端不存在"))?;
    let rows = state
        .db
        .terminal_parameter_versions(&terminal.id, 1)
        .await?;
    let (version, raw, _) = rows
        .first()
        .ok_or_else(|| AppError::not_found("没有可重发的参数版本"))?;
    let items: Vec<Value> = serde_json::from_str(raw)
        .map_err(|error| AppError::internal(format!("参数版本 JSON 损坏: {error}")))?;
    let parameters: Vec<jt808::TerminalParameter> = items
        .into_iter()
        .map(|item| {
            let id = item
                .get("id")
                .and_then(Value::as_u64)
                .and_then(|id| u32::try_from(id).ok())
                .ok_or_else(|| AppError::internal("参数版本缺少 id"))?;
            let value_hex = item
                .get("value_hex")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::internal("参数版本缺少 value_hex"))?;
            Ok(jt808::TerminalParameter {
                id,
                value: decode_hex(value_hex)?,
            })
        })
        .collect::<AppResult<_>>()?;
    let body = jt808::encode_terminal_parameters(&parameters)
        .map_err(|error| AppError::bad_request(format!("参数体错误: {error}")))?;
    let serial = send(state, phone, jt808::MSG_SET_PARAMETERS, body).await?;
    let json = serde_json::json!(parameters
        .iter()
        .map(|parameter| json!({
            "id": parameter.id,
            "value_hex": parameter
                .value
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<String>()
        }))
        .collect::<Vec<_>>())
    .to_string();
    if let Err(error) = state
        .db
        .save_terminal_parameter_version(&terminal.id, &json)
        .await
    {
        tracing::warn!(%error, %phone, "save retried parameter version failed");
    }
    Ok((serial, *version))
}

pub async fn retry_parameters(
    State(state): State<AppState>,
    user: AuthUser,
    Path(phone): Path<String>,
) -> AppResult<Json<Value>> {
    user.require_write()?;
    ensure_terminal(&state, &phone).await?;
    let (serial, version) = retry_latest_parameters(&state, &phone).await?;
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "terminal.retry_parameters",
            &phone,
            &format!("重发参数版本 v{version}"),
        )
        .await?;
    Ok(Json(json!({
        "serial": serial,
        "msg_id": jt808::MSG_SET_PARAMETERS,
        "version": version
    })))
}

#[cfg(test)]
mod tests {
    use super::diff_parameter_sets;

    #[test]
    fn parameter_diff_reports_added_changed_removed() {
        let old = r#"[{"id":1,"value_hex":"00"},{"id":2,"value_hex":"01"}]"#;
        let new = r#"[{"id":2,"value_hex":"02"},{"id":3,"value_hex":"03"}]"#;
        let diff = diff_parameter_sets(1, 2, old, new);
        let changes = diff["changes"].as_array().expect("changes");
        assert_eq!(changes.len(), 3);
        assert!(changes.iter().any(|change| change["change"] == "added"));
        assert!(changes.iter().any(|change| change["change"] == "changed"));
        assert!(changes.iter().any(|change| change["change"] == "removed"));
    }
}
