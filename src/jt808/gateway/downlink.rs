//! Terminal 0x0001 acknowledgement handling.

use super::*;

pub(super) async fn ack(
    state: &AppState,
    frame: &Frame,
    server_serial: u16,
    authenticated: bool,
) -> Option<Vec<u8>> {
    if !authenticated {
        return general_response(frame, 1, server_serial).ok();
    }
    if frame.body.len() < 3 {
        return general_response(frame, 1, server_serial).ok();
    }
    let response_serial = u16::from_be_bytes([frame.body[0], frame.body[1]]);
    let result = frame.body[2];
    complete_downlink(&frame.terminal_id, response_serial, result);
    acknowledge_temp_tracking(&frame.terminal_id, response_serial);
    acknowledge_link_test(&frame.terminal_id, response_serial);
    let Some(command_id) = take_pending_command(&frame.terminal_id, response_serial) else {
        return general_response(frame, 0, server_serial).ok();
    };
    let status = if result == 0 { "success" } else { "failed" };
    let message = if result == 0 {
        "终端已执行指令".to_string()
    } else {
        format!("终端拒绝指令，结果码 {result}")
    };
    if let Ok(Some(done)) = state
        .db
        .set_command_status(&command_id, status, Some(message))
        .await
    {
        state.bus.publish(RealtimeEvent::Command { command: done });
    }
    general_response(frame, 0, server_serial).ok()
}
