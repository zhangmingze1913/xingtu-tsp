//! Registration, authentication and heartbeat handling.

use super::*;

pub(super) async fn register(
    state: &AppState,
    frame: &Frame,
    server_serial: u16,
) -> Option<Vec<u8>> {
    let registration = match parse_registration(&frame.body) {
        Ok(registration) => registration,
        Err(error) => {
            tracing::debug!(%error, terminal = %frame.terminal_id, "invalid JT808 registration");
            return general_response(frame, 1, server_serial).ok();
        }
    };
    let generated_auth_code = new_auth_code();
    let model = if registration.model.is_empty() {
        registration.manufacturer.clone()
    } else {
        registration.model.clone()
    };
    match state
        .db
        .register_jt_terminal(&frame.terminal_id, &model, &generated_auth_code)
        .await
    {
        Ok(terminal) => {
            let registration_json = serde_json::json!({
                "layout": &registration.layout,
                "province": registration.province,
                "city": registration.city,
                "manufacturer": &registration.manufacturer,
                "model": &registration.model,
                "terminal_id": &registration.terminal_id,
                "plate_color": registration.plate_color,
                "plate_no": &registration.plate_no
            })
            .to_string();
            if let Err(error) = state
                .db
                .save_terminal_registration(&terminal.id, &registration_json)
                .await
            {
                tracing::warn!(%error, terminal = %frame.terminal_id, "save JT808 registration failed");
            }
            // 0x0100 only registers the terminal and returns 0x8100.
            // It must NOT authenticate the connection; 0x0102 is the
            // authentication step in JT/T 808-2019.
            let code = terminal.auth_code.unwrap_or(generated_auth_code);
            tracing::info!(terminal = %frame.terminal_id, "JT808 terminal registered, awaiting authentication");
            register_response(frame, &code, server_serial).ok()
        }
        Err(error) => {
            tracing::warn!(%error, "failed to persist JT808 terminal registration");
            general_response(frame, 1, server_serial).ok()
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn auth(
    state: &AppState,
    frame: &Frame,
    server_serial: u16,
    authenticated: &mut bool,
    peer: &str,
    sessions: &mut HashSet<String>,
    out_tx: &mpsc::Sender<Vec<u8>>,
    cancel: &Arc<AtomicBool>,
) -> Option<Vec<u8>> {
    let code = String::from_utf8_lossy(&frame.body)
        .trim_matches('\0')
        .trim()
        .to_string();
    match state
        .db
        .verify_terminal_auth(&frame.terminal_id, &code)
        .await
    {
        Ok(true) => {
            *authenticated = true;
            if !sessions.contains(frame.terminal_id.as_str()) {
                sessions.insert(frame.terminal_id.clone());
            }
            register_terminal_sender(&frame.terminal_id, out_tx.clone(), cancel.clone());
            state
                .cluster
                .register_session(&frame.terminal_id, peer)
                .await;
            let owner = state.cluster.node_id().to_string();
            state
                .cluster
                .publish_jt808_kick(&frame.terminal_id, &owner)
                .await;
            general_response(frame, 0, server_serial).ok()
        }
        Ok(false) => {
            state
                .metrics
                .jt808_auth_failures
                .fetch_add(1, Ordering::Relaxed);
            general_response(frame, 1, server_serial).ok()
        }
        Err(error) => {
            tracing::warn!(%error, "JT808 auth lookup failed");
            general_response(frame, 1, server_serial).ok()
        }
    }
}

pub(super) async fn heartbeat(
    state: &AppState,
    frame: &Frame,
    server_serial: u16,
    authenticated: bool,
    peer: &str,
    sessions: &mut HashSet<String>,
    _out_tx: &mpsc::Sender<Vec<u8>>,
) -> Option<Vec<u8>> {
    if !authenticated {
        return general_response(frame, 1, server_serial).ok();
    }
    let _ = state.db.heartbeat_terminal(&frame.terminal_id).await;
    if !sessions.contains(frame.terminal_id.as_str()) {
        sessions.insert(frame.terminal_id.clone());
    }
    state
        .cluster
        .register_session(&frame.terminal_id, peer)
        .await;
    general_response(frame, 0, server_serial).ok()
}
