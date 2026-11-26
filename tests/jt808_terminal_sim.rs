//! Simulated JT/T 808 terminal over a real TCP connection.
//!
//! This replaces a hardware terminal only for protocol/edge-path testing. It
//! does not prove RF, GPS, vehicle-bus, or device-firmware behavior.

mod common;

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use xingtu_tsp::{
    db::Db,
    jt808::{self, build_frame, parse_frame, Frame},
    models::CreateVehicleRequest,
    state::AppState,
};

const PHONE: &str = "013800000001";

async fn read_frame(stream: &mut TcpStream) -> Frame {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let read = tokio::time::timeout(Duration::from_secs(3), stream.read(&mut chunk))
            .await
            .expect("read timeout")
            .expect("read");
        assert!(read > 0, "server closed connection");
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.iter().filter(|byte| **byte == 0x7e).count() >= 2 {
            return parse_frame(&buffer).expect("parse response");
        }
    }
}

fn registration_body() -> Vec<u8> {
    let mut body = Vec::with_capacity(37);
    body.extend_from_slice(&0x0031u16.to_be_bytes()); // province
    body.extend_from_slice(&0x0001u16.to_be_bytes()); // city
    body.extend_from_slice(b"MANUF"); // 5
    body.extend_from_slice(b"MODEL-2026          "); // 20
    body.extend_from_slice(&[0x01, 0x38, 0x00, 0x00, 0x00, 0x01, 0x01]); // 7-byte terminal id
    body.push(1); // plate color
    assert_eq!(body.len(), 37);
    body
}

async fn send_and_receive(stream: &mut TcpStream, raw: Vec<u8>) -> Frame {
    stream.write_all(&raw).await.expect("write");
    read_frame(stream).await
}

async fn register_and_auth(stream: &mut TcpStream) -> String {
    let raw = build_frame(jt808::MSG_TERMINAL_REGISTER, PHONE, 1, &registration_body())
        .expect("register frame");
    let response = send_and_receive(stream, raw).await;
    assert_eq!(response.msg_id, jt808::MSG_REGISTER_RESPONSE);
    assert_eq!(response.body[0..2], 1u16.to_be_bytes());
    assert_eq!(response.body[2], 0);
    let auth_code = String::from_utf8_lossy(&response.body[3..])
        .trim_matches('\0')
        .to_string();
    assert!(!auth_code.is_empty());

    let raw =
        build_frame(jt808::MSG_TERMINAL_AUTH, PHONE, 2, auth_code.as_bytes()).expect("auth frame");
    let response = send_and_receive(stream, raw).await;
    assert_eq!(response.msg_id, jt808::MSG_GENERAL_RESPONSE);
    assert_eq!(response.body[2], 0, "auth rejected");
    auth_code
}

#[tokio::test]
async fn simulated_terminal_register_auth_heartbeat_location_reconnect() {
    let probe = TcpListener::bind("127.0.0.1:0").await.expect("probe bind");
    let addr = probe.local_addr().expect("probe addr");
    drop(probe);

    let mut config = common::test_config();
    config.jt808_addr = addr.to_string();
    config.jt808_allow_plaintext = true;
    let db = Db::in_memory().await.expect("db");
    let state = AppState::new(config, db.clone());
    let server = tokio::spawn(async move { jt808::serve(state).await });

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let auth_code = register_and_auth(&mut stream).await;

    let vehicle = db
        .create_vehicle(CreateVehicleRequest {
            vin: "LSVAA2A41NX808001".to_string(),
            plate_no: "娌狝80801".to_string(),
            model: "SIM-808".to_string(),
            color: "white".to_string(),
            energy_type: "EV".to_string(),
            driver_name: "Sim Driver".to_string(),
            driver_phone: "13600008080".to_string(),
            org_name: "SimOrg".to_string(),
        })
        .await
        .expect("vehicle");
    let terminal = db
        .get_terminal_by_phone(PHONE)
        .await
        .expect("terminal lookup")
        .expect("terminal registered");
    db.bind_terminal(&terminal.id, Some(vehicle.vehicle.id.clone()))
        .await
        .expect("bind terminal");

    let raw = build_frame(jt808::MSG_HEARTBEAT, PHONE, 3, &[]).expect("heartbeat");
    let response = send_and_receive(&mut stream, raw).await;
    assert_eq!(response.body[2], 0, "heartbeat rejected");

    let body = jt808::codec::encode_location_response(
        31.2304,
        121.4737,
        42.0,
        90.0,
        true,
        "2026-11-26T15:00:00+08:00",
    );
    let raw = build_frame(jt808::MSG_LOCATION_REPORT, PHONE, 4, &body).expect("location");
    let response = send_and_receive(&mut stream, raw).await;
    assert_eq!(response.msg_id, jt808::MSG_GENERAL_RESPONSE);
    assert_eq!(response.body[2], 0, "location rejected");

    // Duplicate report with the same timestamp must not create a second row.
    let raw = build_frame(jt808::MSG_LOCATION_REPORT, PHONE, 5, &body).expect("duplicate");
    let response = send_and_receive(&mut stream, raw).await;
    assert_eq!(response.body[2], 0, "duplicate rejected");
    assert_eq!(
        db.count_telemetry_for(&vehicle.vehicle.id)
            .await
            .expect("count telemetry"),
        1,
        "duplicate location inserted twice"
    );

    drop(stream);

    // Reconnect with the same auth code (device reboot / network flap).
    let mut stream = TcpStream::connect(addr).await.expect("reconnect");
    let code_again = register_and_auth(&mut stream).await;
    assert_eq!(code_again, auth_code);
    let raw = build_frame(jt808::MSG_HEARTBEAT, PHONE, 5, &[]).expect("heartbeat 2");
    let response = send_and_receive(&mut stream, raw).await;
    assert_eq!(response.body[2], 0, "heartbeat after reconnect rejected");

    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn fragmented_frame_is_reassembled() {
    let probe = TcpListener::bind("127.0.0.1:0").await.expect("probe");
    let addr = probe.local_addr().expect("addr");
    drop(probe);

    let mut config = common::test_config();
    config.jt808_addr = addr.to_string();
    config.jt808_allow_plaintext = true;
    let state = AppState::new(config, Db::in_memory().await.expect("db"));
    let server = tokio::spawn(async move { jt808::serve(state).await });

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let raw = build_frame(
        jt808::MSG_TERMINAL_REGISTER,
        "013800000099",
        1,
        &registration_body(),
    )
    .expect("frame");
    let split = raw.len() / 2;
    stream.write_all(&raw[..split]).await.expect("first half");
    tokio::time::sleep(Duration::from_millis(100)).await;
    stream.write_all(&raw[split..]).await.expect("second half");
    let response = read_frame(&mut stream).await;
    assert_eq!(response.msg_id, jt808::MSG_REGISTER_RESPONSE);
    assert_eq!(response.body[2], 0, "fragmented register rejected");

    server.abort();
    let _ = server.await;
}
