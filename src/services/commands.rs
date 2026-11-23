use std::time::Duration;

use base64::Engine as _;

use crate::{models::Command, realtime::RealtimeEvent, state::AppState};

fn success_message(command: &str) -> &'static str {
    match command {
        "lock" => "车辆已成功上锁",
        "unlock" => "车辆已成功解锁",
        "start" => "远程启动成功，空调将按预设开启",
        "stop" => "车辆已安全熄火",
        "flash" => "车辆灯光已闪烁",
        "honk" => "车辆喇叭已鸣响",
        "ac_on" => "空调已开启，目标温度 24°C",
        "ac_off" => "空调已关闭",
        "charge_start" => "充电已启动，当前功率 60 kW",
        "charge_stop" => "充电已停止",
        "window_close" => "全部车窗已关闭",
        "find" => "车辆已鸣笛并闪灯，等待车主确认",
        _ => "指令执行成功",
    }
}

fn failure_message(command: &str) -> &'static str {
    match command {
        "start" => "车辆防盗策略拒绝启动，请检查钥匙状态",
        "charge_start" => "充电桩握手失败，请稍后重试",
        _ => "T-Box 响应超时，指令未执行",
    }
}

pub fn dispatch(state: AppState, command: Command) {
    tokio::spawn(async move {
        if state.config.command_mode == "jt808" {
            let terminal = match state.db.get_terminal_by_vehicle(&command.vehicle_id).await {
                Ok(Some(terminal)) => terminal,
                _ => {
                    fail(&state, &command.id, "车辆未绑定 JT/T 808 终端").await;
                    return;
                }
            };
            match crate::jt808::send_command(
                &terminal.phone,
                &command.command,
                command.payload.as_deref(),
                &command.id,
            ) {
                Ok(serial) => {
                    tracing::info!(
                        command = %command.id,
                        terminal = %terminal.phone,
                        serial,
                        "JT/T 808 command sent"
                    );
                    if let Ok(Some(sent)) =
                        state.db.set_command_status(&command.id, "sent", None).await
                    {
                        state.bus.publish(RealtimeEvent::Command { command: sent });
                    }
                    let timeout_state = state.clone();
                    let id = command.id.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(15)).await;
                        if let Ok(Some(current)) = timeout_state.db.get_command(&id).await {
                            if current.status == "sent" {
                                fail(&timeout_state, &id, "终端应答超时").await;
                            }
                        }
                    });
                }
                Err(error) => {
                    let session = state.cluster.lookup_session(&terminal.phone).await;
                    let remote = session
                        .as_ref()
                        .filter(|session| session.node_id != state.cluster.node_id());
                    let Some(session) = remote else {
                        fail(&state, &command.id, &error).await;
                        return;
                    };
                    let serial = crate::jt808::next_command_serial();
                    let frame = match crate::jt808::build_command_frame(
                        &terminal.phone,
                        &command.command,
                        command.payload.as_deref(),
                        serial,
                    ) {
                        Ok(frame) => frame,
                        Err(error) => {
                            fail(&state, &command.id, &error).await;
                            return;
                        }
                    };
                    let payload = serde_json::json!({
                        "phone": terminal.phone.clone(),
                        "serial": serial,
                        "command_id": command.id.clone(),
                        "frame": base64::engine::general_purpose::STANDARD.encode(frame),
                    })
                    .to_string();
                    if !state
                        .cluster
                        .publish_gateway(&session.node_id, &payload)
                        .await
                    {
                        fail(&state, &command.id, "跨节点下发失败").await;
                        return;
                    }
                    tracing::info!(
                        command = %command.id,
                        terminal = %terminal.phone,
                        node = %session.node_id,
                        serial,
                        "JT/T 808 command routed to gateway replica"
                    );
                    if let Ok(Some(sent)) =
                        state.db.set_command_status(&command.id, "sent", None).await
                    {
                        state.bus.publish(RealtimeEvent::Command { command: sent });
                    }
                    let timeout_state = state.clone();
                    let id = command.id.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(15)).await;
                        if let Ok(Some(current)) = timeout_state.db.get_command(&id).await {
                            if current.status == "sent" {
                                fail(&timeout_state, &id, "终端应答超时").await;
                            }
                        }
                    });
                }
            }
            return;
        }

        if state.config.command_mode != "simulated" {
            fail(
                &state,
                &command.id,
                "指令通道已禁用（TSP_COMMAND_MODE=disabled）",
            )
            .await;
            return;
        }

        tokio::time::sleep(Duration::from_millis(900)).await;
        if let Ok(Some(sent)) = state.db.set_command_status(&command.id, "sent", None).await {
            state.bus.publish(RealtimeEvent::Command { command: sent });
        }

        tokio::time::sleep(Duration::from_millis(1400)).await;
        let success = rand::random::<f64>() > 0.08;
        let (status, result) = if success {
            ("success", success_message(&command.command).to_string())
        } else {
            ("failed", failure_message(&command.command).to_string())
        };
        if let Ok(Some(done)) = state
            .db
            .set_command_status(&command.id, status, Some(result))
            .await
        {
            state.bus.publish(RealtimeEvent::Command { command: done });
        }
    });
}

async fn fail(state: &AppState, id: &str, message: &str) {
    if let Ok(Some(done)) = state
        .db
        .set_command_status(id, "failed", Some(message.to_string()))
        .await
    {
        state.bus.publish(RealtimeEvent::Command { command: done });
    }
}
