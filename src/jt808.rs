//! JT/T 808-2019 edge-gateway subset.
//!
//! Supported messages: terminal registration (0x0100), authentication (0x0102),
//! heartbeat (0x0002), location report (0x0200) and the platform general response
//! (0x8001). This is deliberately a subset, not a full JT/T 808-2019
//! conformance implementation: sub-package and encrypted bodies are rejected,
//! and the full 2019 command/parameter/area/video message set is not present.

use std::{
    collections::{hash_map::DefaultHasher, HashMap, HashSet},
    hash::{Hash, Hasher},
    sync::{
        atomic::{AtomicBool, AtomicU16, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Instant,
};

use rand::{distributions::Alphanumeric, Rng};
use serde::Serialize;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};

pub mod codec;
mod control;
pub use codec::*;
pub use control::*;

use crate::{
    models::LocationSample, realtime::RealtimeEvent, services, state::AppState, util::now_iso,
};

const SESSION_SHARDS: usize = 64;
const SESSION_OUTBOX_CAPACITY: usize = 32;

type TerminalSender = mpsc::Sender<Vec<u8>>;

struct TerminalSession {
    sender: TerminalSender,
    cancel: Arc<AtomicBool>,
}

struct SessionShard {
    sessions: Mutex<HashMap<String, TerminalSession>>,
}

fn session_shards() -> &'static [SessionShard] {
    static SHARDS: OnceLock<Vec<SessionShard>> = OnceLock::new();
    SHARDS.get_or_init(|| {
        (0..SESSION_SHARDS)
            .map(|_| SessionShard {
                sessions: Mutex::new(HashMap::new()),
            })
            .collect()
    })
}

/// 64-way sharded registry: a flush/registration on one terminal no longer
/// serializes all million connections behind a single global Mutex.
fn sessions_for(phone: &str) -> &'static Mutex<HashMap<String, TerminalSession>> {
    let mut hasher = DefaultHasher::new();
    phone.hash(&mut hasher);
    &session_shards()[hasher.finish() as usize % SESSION_SHARDS].sessions
}

#[derive(Debug, Clone, Serialize)]
pub struct TempTrackingStatus {
    pub phone: String,
    pub serial: u16,
    pub interval_seconds: u16,
    pub expires_at: i64,
    pub acknowledged: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinkTestStatus {
    pub phone: String,
    pub serial: u16,
    pub sent_at: i64,
    pub acknowledged: bool,
}

fn link_tests() -> &'static Mutex<HashMap<String, LinkTestStatus>> {
    static TESTS: OnceLock<Mutex<HashMap<String, LinkTestStatus>>> = OnceLock::new();
    TESTS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn register_link_test(phone: &str, serial: u16) {
    if let Ok(mut tests) = link_tests().lock() {
        tests.insert(
            phone.to_string(),
            LinkTestStatus {
                phone: phone.to_string(),
                serial,
                sent_at: chrono::Utc::now().timestamp(),
                acknowledged: false,
            },
        );
    }
}

pub fn acknowledge_link_test(phone: &str, serial: u16) {
    if let Ok(mut tests) = link_tests().lock() {
        if let Some(status) = tests.get_mut(phone) {
            if status.serial == serial {
                status.acknowledged = true;
            }
        }
    }
}

pub fn link_test_status(phone: &str) -> Option<LinkTestStatus> {
    link_tests().lock().ok()?.get(phone).cloned()
}

fn temp_tracking() -> &'static Mutex<HashMap<String, TempTrackingStatus>> {
    static TRACKING: OnceLock<Mutex<HashMap<String, TempTrackingStatus>>> = OnceLock::new();
    TRACKING.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn register_temp_tracking(
    phone: &str,
    serial: u16,
    interval_seconds: u16,
    expiry_seconds: u32,
) {
    if let Ok(mut tracking) = temp_tracking().lock() {
        tracking.insert(
            phone.to_string(),
            TempTrackingStatus {
                phone: phone.to_string(),
                serial,
                interval_seconds,
                expires_at: chrono::Utc::now().timestamp() + i64::from(expiry_seconds),
                acknowledged: false,
            },
        );
    }
}

pub fn acknowledge_temp_tracking(phone: &str, serial: u16) {
    if let Ok(mut tracking) = temp_tracking().lock() {
        if let Some(status) = tracking.get_mut(phone) {
            if status.serial == serial {
                status.acknowledged = true;
            }
        }
    }
}

pub fn temp_tracking_status(phone: &str) -> Option<TempTrackingStatus> {
    let mut tracking = temp_tracking().lock().ok()?;
    let status = tracking.get(phone)?.clone();
    if status.expires_at <= chrono::Utc::now().timestamp() {
        tracking.remove(phone);
        return None;
    }
    Some(status)
}

fn batch_windows() -> &'static Mutex<HashMap<String, std::collections::VecDeque<Instant>>> {
    static WINDOWS: OnceLock<Mutex<HashMap<String, std::collections::VecDeque<Instant>>>> =
        OnceLock::new();
    WINDOWS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Per-terminal 0x0704 rate limit: at most 20 batches in a 10 second window.
pub fn allow_location_batch(phone: &str) -> bool {
    let Ok(mut windows) = batch_windows().lock() else {
        return false;
    };
    let now = Instant::now();
    let window = windows.entry(phone.to_string()).or_default();
    while window
        .front()
        .is_some_and(|started| now.duration_since(*started) > std::time::Duration::from_secs(10))
    {
        window.pop_front();
    }
    if window.len() >= 20 {
        return false;
    }
    window.push_back(now);
    true
}

fn pending_commands() -> &'static Mutex<HashMap<String, String>> {
    static PENDING: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Debug, Clone, Serialize)]
pub struct DownlinkStatus {
    pub phone: String,
    pub serial: u16,
    pub msg_id: u16,
    pub status: String,
    pub result: Option<u8>,
    pub sent_at: String,
    pub completed_at: Option<String>,
}

fn downlinks() -> &'static Mutex<HashMap<String, DownlinkStatus>> {
    static DOWNLINKS: OnceLock<Mutex<HashMap<String, DownlinkStatus>>> = OnceLock::new();
    DOWNLINKS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn track_downlink(phone: &str, msg_id: u16, serial: u16) {
    if let Ok(mut links) = downlinks().lock() {
        if links.len() > 2048 {
            links.clear();
        }
        links.insert(
            format!("{phone}:{serial}"),
            DownlinkStatus {
                phone: phone.to_string(),
                serial,
                msg_id,
                status: "pending".to_string(),
                result: None,
                sent_at: now_iso(),
                completed_at: None,
            },
        );
    }
}

pub fn complete_downlink(phone: &str, serial: u16, result: u8) -> Option<DownlinkStatus> {
    let mut links = downlinks().lock().ok()?;
    let status = links.get_mut(&format!("{phone}:{serial}"))?;
    status.status = if result == 0 { "success" } else { "failed" }.to_string();
    status.result = Some(result);
    status.completed_at = Some(now_iso());
    Some(status.clone())
}

/// Mark pending downlinks older than `older_than_secs` as timed out. This is a
/// detection/observability step; it does not silently resend a command.
pub fn expire_stale_downlinks(older_than_secs: i64) -> Vec<DownlinkStatus> {
    let Ok(mut links) = downlinks().lock() else {
        return Vec::new();
    };
    let now = chrono::Utc::now();
    let mut expired = Vec::new();
    for status in links.values_mut() {
        if status.status != "pending" {
            continue;
        }
        let Ok(sent) = chrono::DateTime::parse_from_rfc3339(&status.sent_at) else {
            continue;
        };
        if (now - sent.with_timezone(&chrono::Utc)).num_seconds() > older_than_secs {
            status.status = "timeout".to_string();
            status.completed_at = Some(now_iso());
            expired.push(status.clone());
        }
    }
    expired
}

pub fn downlink_status(phone: &str, serial: u16) -> Option<DownlinkStatus> {
    downlinks()
        .lock()
        .ok()?
        .get(&format!("{phone}:{serial}"))
        .cloned()
}

fn command_serial() -> &'static AtomicU16 {
    static SERIAL: OnceLock<AtomicU16> = OnceLock::new();
    SERIAL.get_or_init(|| AtomicU16::new(1000))
}

pub fn register_terminal_sender(phone: &str, sender: TerminalSender, cancel: Arc<AtomicBool>) {
    if let Ok(mut sessions) = sessions_for(phone).lock() {
        if let Some(previous) = sessions.insert(
            phone.to_string(),
            TerminalSession {
                sender,
                cancel: cancel.clone(),
            },
        ) {
            // Kick the previous TCP connection for this terminal.
            previous.cancel.store(true, Ordering::Relaxed);
        }
    }
}

/// Cancel a locally connected terminal. Cross-node sessions receive this via
/// the Redis kick channel; the old connection closes on its next read/frame.
pub fn cancel_terminal_session(phone: &str) {
    if let Ok(sessions) = sessions_for(phone).lock() {
        if let Some(session) = sessions.get(phone) {
            session.cancel.store(true, Ordering::Relaxed);
        }
    }
}

pub fn unregister_terminal_sender(phone: &str, cancel: &Arc<AtomicBool>) {
    if let Ok(mut sessions) = sessions_for(phone).lock() {
        if sessions
            .get(phone)
            .is_some_and(|session| Arc::ptr_eq(&session.cancel, cancel))
        {
            sessions.remove(phone);
        }
    }
}

/// Send a JT/T 808 0x8300 text command to a terminal connected to this gateway
/// process. The returned serial is used to correlate 0x0001 responses.
pub fn send_text_command(
    phone: &str,
    command: &str,
    payload: Option<&str>,
    command_id: &str,
) -> Result<u16, String> {
    let sender = sessions_for(phone)
        .lock()
        .map_err(|_| "JT808 session registry poisoned".to_string())?
        .get(phone)
        .map(|session| session.sender.clone())
        .ok_or_else(|| "终端未连接当前网关".to_string())?;
    let serial = command_serial().fetch_add(1, Ordering::Relaxed);
    let frame = build_text_frame(phone, command, payload, serial)?;
    register_pending_command(phone, serial, command_id);
    sender
        .try_send(frame)
        .map_err(|_| "终端连接已断开".to_string())?;
    Ok(serial)
}

/// Send a command using the correct downlink message: vehicle-control actions
/// use 0x8500; other commands fall back to 0x8300 text.
pub fn send_command(
    phone: &str,
    command: &str,
    payload: Option<&str>,
    command_id: &str,
) -> Result<u16, String> {
    let sender = sessions_for(phone)
        .lock()
        .map_err(|_| "JT808 session registry poisoned".to_string())?
        .get(phone)
        .map(|session| session.sender.clone())
        .ok_or_else(|| "终端未连接当前网关".to_string())?;
    let serial = command_serial().fetch_add(1, Ordering::Relaxed);
    let frame = build_command_frame(phone, command, payload, serial)?;
    register_pending_command(phone, serial, command_id);
    sender
        .try_send(frame)
        .map_err(|_| "终端连接已断开".to_string())?;
    Ok(serial)
}

/// Build the downlink frame for a command. 0x8500 for control actions, 0x8300
/// text for everything else.
pub fn build_command_frame(
    phone_id: &str,
    command: &str,
    payload: Option<&str>,
    serial: u16,
) -> Result<Vec<u8>, String> {
    if command == "raw_808" {
        let payload = payload.unwrap_or_default();
        let (id, hex) = payload
            .split_once(':')
            .ok_or_else(|| "raw_808 格式应为 msg_id:hex_body".to_string())?;
        let msg_id = u16::from_str_radix(id.trim().trim_start_matches("0x"), 16)
            .map_err(|_| "raw_808 msg_id 非法".to_string())?;
        let body = decode_hex(hex.trim())?;
        return build_frame(msg_id, phone_id, serial, &body)
            .map_err(|error| format!("构建 raw_808 失败: {error}"));
    }
    if command == "info_service" {
        let payload = payload.unwrap_or_default();
        let (info_type, content) = payload
            .split_once(':')
            .and_then(|(kind, text)| kind.parse::<u8>().ok().map(|kind| (kind, text)))
            .unwrap_or((0, payload));
        let body = encode_info_service(info_type, content)
            .map_err(|_| "0x8304 信息服务内容为空或超过 1024 字节".to_string())?;
        return build_frame(MSG_INFO_SERVICE, phone_id, serial, &body)
            .map_err(|error| format!("构建 0x8304 失败: {error}"));
    }
    if matches!(command, "phone_call" | "callback") {
        let phone = payload.unwrap_or_default().trim();
        let body = encode_phone_call(0, phone)
            .map_err(|_| "0x8400 电话号码必须为 1-32 位数字".to_string())?;
        return build_frame(MSG_PHONE_CALL, phone_id, serial, &body)
            .map_err(|error| format!("构建 0x8400 失败: {error}"));
    }
    if let Some((flag, parameter)) = vehicle_control_action(command) {
        let body = encode_vehicle_control(flag, &parameter);
        return build_frame(MSG_VEHICLE_CONTROL, phone_id, serial, &body)
            .map_err(|error| format!("构建 0x8500 失败: {error}"));
    }
    build_text_frame(phone_id, command, payload, serial)
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) {
        return Err("raw_808 body 必须是偶数位十六进制".to_string());
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    let bytes = value.as_bytes();
    for pair in bytes.chunks(2) {
        let text = std::str::from_utf8(pair).map_err(|_| "raw_808 body 非 ASCII".to_string())?;
        out.push(
            u8::from_str_radix(text, 16).map_err(|_| "raw_808 body 十六进制非法".to_string())?,
        );
    }
    Ok(out)
}

/// Build a 0x8300 frame without delivering it. Used when the terminal is
/// connected to another gateway replica and delivery goes through Redis.
pub fn build_text_frame(
    phone: &str,
    command: &str,
    payload: Option<&str>,
    serial: u16,
) -> Result<Vec<u8>, String> {
    let text = match payload {
        Some(payload) => format!("{command} {payload}"),
        None => command.to_string(),
    };
    let (gbk, _, had_errors) = encoding_rs::GBK.encode(&text);
    if had_errors {
        return Err("指令文本无法编码为 GBK".to_string());
    }
    let mut body = Vec::with_capacity(gbk.len() + 1);
    body.push(0x01);
    body.extend_from_slice(&gbk[..gbk.len().min(1000)]);
    build_frame(MSG_TEXT_COMMAND, phone, serial, &body)
        .map_err(|error| format!("构建 0x8300 失败: {error}"))
}

pub fn next_command_serial() -> u16 {
    command_serial().fetch_add(1, Ordering::Relaxed)
}

/// Send an arbitrary JT/T 808 downlink frame to a terminal connected to this
/// gateway process. Used by parameter/control/query APIs.
pub fn send_frame(phone: &str, msg_id: u16, body: &[u8]) -> Result<u16, String> {
    let sender = sessions_for(phone)
        .lock()
        .map_err(|_| "JT808 session registry poisoned".to_string())?
        .get(phone)
        .map(|session| session.sender.clone())
        .ok_or_else(|| "终端未连接当前网关".to_string())?;
    let serial = next_command_serial();
    let frame = build_frame(msg_id, phone, serial, body)
        .map_err(|error| format!("构建 0x{msg_id:04x} 失败: {error}"))?;
    sender
        .try_send(frame)
        .map_err(|_| "终端连接已断开".to_string())?;
    Ok(serial)
}

pub fn register_pending_command(phone: &str, serial: u16, command_id: &str) {
    if let Ok(mut pending) = pending_commands().lock() {
        pending.insert(format!("{phone}:{serial}"), command_id.to_string());
    }
}

/// Deliver a frame built by another API replica to a terminal that is connected
/// to this gateway process.
pub fn deliver_local_frame(phone: &str, frame: Vec<u8>) -> bool {
    let Some(sender) = sessions_for(phone)
        .lock()
        .ok()
        .and_then(|sessions| sessions.get(phone).map(|session| session.sender.clone()))
    else {
        return false;
    };
    sender.try_send(frame).is_ok()
}

fn take_pending_command(phone: &str, serial: u16) -> Option<String> {
    pending_commands()
        .lock()
        .ok()?
        .remove(&format!("{phone}:{serial}"))
}

fn general_response(
    frame: &Frame,
    result: u8,
    server_serial: u16,
) -> Result<Vec<u8>, ProtocolError> {
    let mut body = Vec::with_capacity(3);
    body.extend_from_slice(&frame.serial.to_be_bytes());
    body.push(result);
    build_frame(
        MSG_GENERAL_RESPONSE,
        &frame.terminal_id,
        server_serial,
        &body,
    )
}

fn register_response(
    frame: &Frame,
    auth_code: &str,
    server_serial: u16,
) -> Result<Vec<u8>, ProtocolError> {
    let mut body = Vec::with_capacity(3 + auth_code.len());
    body.extend_from_slice(&frame.serial.to_be_bytes());
    body.push(0);
    body.extend_from_slice(auth_code.as_bytes());
    build_frame(
        MSG_REGISTER_RESPONSE,
        &frame.terminal_id,
        server_serial,
        &body,
    )
}

fn next_serial(counter: &AtomicU16) -> u16 {
    counter.fetch_add(1, Ordering::Relaxed)
}

/// Registration issues a fresh, non-guessable auth code. The terminal must
/// still send 0x0102 before the connection is considered authenticated.
fn new_auth_code() -> String {
    let suffix: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(12)
        .map(char::from)
        .collect();
    format!("XT{suffix}")
}

mod gateway;
pub use gateway::serve;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_cancels_old_and_old_cleanup_keeps_new() {
        let phone = "013800009999";
        let (first, _first_rx) = mpsc::channel(8);
        let first_cancel = Arc::new(AtomicBool::new(false));
        register_terminal_sender(phone, first, first_cancel.clone());
        assert!(!first_cancel.load(Ordering::Relaxed));

        let (second, _second_rx) = mpsc::channel(8);
        let second_cancel = Arc::new(AtomicBool::new(false));
        register_terminal_sender(phone, second, second_cancel.clone());
        assert!(first_cancel.load(Ordering::Relaxed));

        unregister_terminal_sender(phone, &first_cancel);
        assert!(sessions_for(phone).lock().unwrap().contains_key(phone));
        unregister_terminal_sender(phone, &second_cancel);
        assert!(!sessions_for(phone).lock().unwrap().contains_key(phone));
    }

    #[test]
    fn bounded_outbox_rejects_when_full() {
        let phone = "013800009993";
        let (sender, _rx) = mpsc::channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        register_terminal_sender(phone, sender, cancel.clone());
        assert!(send_frame(phone, MSG_QUERY_PARAMETERS, &[]).is_ok());
        assert!(send_frame(phone, MSG_QUERY_PARAMETERS, &[]).is_err());
        unregister_terminal_sender(phone, &cancel);
    }

    #[test]
    fn location_batch_rate_limit_window() {
        let phone = "013800009994";
        for _ in 0..20 {
            assert!(allow_location_batch(phone));
        }
        assert!(!allow_location_batch(phone));
    }

    #[test]
    fn stale_downlinks_are_marked_timeout() {
        let phone = "013800009995";
        track_downlink(phone, 0x8103, 991);
        assert!(!expire_stale_downlinks(-1).is_empty());
        let status = downlink_status(phone, 991).expect("status");
        assert_eq!(status.status, "timeout");
    }

    #[test]
    fn link_test_ack() {
        let phone = "013800009996";
        register_link_test(phone, 88);
        let pending = link_test_status(phone).expect("link test");
        assert_eq!(pending.serial, 88);
        assert!(!pending.acknowledged);
        acknowledge_link_test(phone, 88);
        assert!(link_test_status(phone).expect("link test").acknowledged);
    }

    #[test]
    fn temp_tracking_ack_and_expiry() {
        let phone = "013800009997";
        register_temp_tracking(phone, 77, 10, 60);
        let pending = temp_tracking_status(phone).expect("tracking");
        assert_eq!(pending.serial, 77);
        assert!(!pending.acknowledged);
        acknowledge_temp_tracking(phone, 77);
        assert!(temp_tracking_status(phone).expect("tracking").acknowledged);

        register_temp_tracking(phone, 78, 10, 0);
        assert!(temp_tracking_status(phone).is_none());
    }

    #[test]
    fn cancel_terminal_session_sets_connection_token() {
        let phone = "013800009998";
        let (sender, _rx) = mpsc::channel(8);
        let cancel = Arc::new(AtomicBool::new(false));
        register_terminal_sender(phone, sender, cancel.clone());
        cancel_terminal_session(phone);
        assert!(cancel.load(Ordering::Relaxed));
        unregister_terminal_sender(phone, &cancel);
    }

    #[test]
    fn vehicle_control_commands_use_0x8500_and_text_falls_back_to_0x8300() {
        let control = build_command_frame("013800000001", "lock", None, 7).expect("control");
        let frame = parse_frame(&control).expect("parse");
        assert_eq!(frame.msg_id, MSG_VEHICLE_CONTROL);

        let info =
            build_command_frame("013800000001", "info_service", Some("2:hello"), 10).expect("info");
        let frame = parse_frame(&info).expect("parse");
        assert_eq!(frame.msg_id, MSG_INFO_SERVICE);
        assert_eq!(
            decode_info_service(&frame.body).unwrap(),
            (2, "hello".to_string())
        );

        let call = build_command_frame("013800000001", "phone_call", Some("13800138000"), 9)
            .expect("call");
        let frame = parse_frame(&call).expect("parse");
        assert_eq!(frame.msg_id, MSG_PHONE_CALL);
        assert_eq!(decode_phone_call(&frame.body).unwrap().1, "13800138000");

        let text = build_command_frame("013800000001", "reboot", Some("now"), 8).expect("text");
        let frame = parse_frame(&text).expect("parse");
        assert_eq!(frame.msg_id, MSG_TEXT_COMMAND);
    }
}
