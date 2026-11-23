//! 0x0104/0x0107 terminal response persistence.

use super::*;

pub(super) async fn parameters(state: &AppState, frame: &Frame) -> u8 {
    let params = match decode_terminal_parameters(&frame.body) {
        Ok(params) => params,
        Err(error) => {
            tracing::debug!(%error, terminal = %frame.terminal_id, "invalid 0x0104 body");
            return 3;
        }
    };
    let Some(terminal) = state
        .db
        .get_terminal_by_phone(&frame.terminal_id)
        .await
        .ok()
        .flatten()
    else {
        return 1;
    };
    let values: Vec<_> = params
        .into_iter()
        .map(|parameter| {
            serde_json::json!({
                "id": parameter.id,
                "value_hex": hex(&parameter.value)
            })
        })
        .collect();
    let json = serde_json::json!(values).to_string();
    if let Err(error) = state
        .db
        .save_terminal_parameter_version(&terminal.id, &json)
        .await
    {
        tracing::warn!(%error, "save parameter version failed");
    }
    match state.db.save_terminal_parameters(&terminal.id, &json).await {
        Ok(()) => 0,
        Err(error) => {
            tracing::warn!(%error, "save 0x0104 parameters failed");
            3
        }
    }
}

pub(super) async fn attributes(state: &AppState, frame: &Frame) -> u8 {
    let Some(terminal) = state
        .db
        .get_terminal_by_phone(&frame.terminal_id)
        .await
        .ok()
        .flatten()
    else {
        return 1;
    };
    let json = match parse_terminal_attributes(&frame.body) {
        Ok(attributes) => serde_json::json!({
            "layout": attributes.layout,
            "terminal_type": attributes.terminal_type,
            "manufacturer": attributes.manufacturer,
            "model": attributes.model,
            "terminal_id": attributes.terminal_id,
            "iccid": attributes.iccid,
            "hardware_version": attributes.hardware_version,
            "firmware_version": attributes.firmware_version,
            "gnss": attributes.gnss,
            "raw_hex": attributes.raw_hex,
            "length": frame.body.len()
        })
        .to_string(),
        Err(_) => serde_json::json!({
            "layout": "raw",
            "raw_hex": hex(&frame.body),
            "length": frame.body.len()
        })
        .to_string(),
    };
    match state.db.save_terminal_attributes(&terminal.id, &json).await {
        Ok(()) => 0,
        Err(error) => {
            tracing::warn!(%error, "save 0x0107 attributes failed");
            3
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02X}"));
    }
    out
}
