//! Minimal JT/T 808 terminal simulator for the bundled gateway.
//!
//! Usage:
//!   cargo run --bin jt808-sim
//!   TSP_JT808_ADDR=127.0.0.1:18080 TSP_JT808_PHONE=013800000001 \
//!   TSP_JT808_AUTH=XT000001 cargo run --bin jt808-sim
//!
//! The default credentials belong to the first seeded demo terminal (沪A·D8F21).

use std::time::Duration;

use chrono::{FixedOffset, Utc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use xingtu_tsp::{build_frame, parse_frame, MSG_LOCATION_REPORT, MSG_TERMINAL_AUTH};

fn bcd_time() -> [u8; 6] {
    let offset = FixedOffset::east_opt(8 * 3600).expect("offset");
    let local = Utc::now().with_timezone(&offset);
    let text = local.format("%y%m%d%H%M%S").to_string();
    let bytes = text.as_bytes();
    let mut out = [0u8; 6];
    for index in 0..6 {
        let high = bytes[index * 2] - b'0';
        let low = bytes[index * 2 + 1] - b'0';
        out[index] = (high << 4) | low;
    }
    out
}

fn location_body(step: u32) -> Vec<u8> {
    let lat = 31.2304 + step as f64 * 0.0004;
    let lon = 121.4737 + step as f64 * 0.0006;
    let mut body = Vec::with_capacity(28);
    body.extend_from_slice(&0u32.to_be_bytes()); // alarm
    body.extend_from_slice(&1u32.to_be_bytes()); // ACC on + positioned
    body.extend_from_slice(&((lat * 1_000_000.0) as u32).to_be_bytes());
    body.extend_from_slice(&((lon * 1_000_000.0) as u32).to_be_bytes());
    body.extend_from_slice(&12u16.to_be_bytes()); // altitude
    body.extend_from_slice(&((42.5 * 10.0) as u16).to_be_bytes()); // speed 42.5 km/h
    body.extend_from_slice(&45u16.to_be_bytes()); // heading
    body.extend_from_slice(&bcd_time());
    body
}

async fn exchange(
    stream: &mut TcpStream,
    msg_id: u16,
    phone: &str,
    serial: u16,
    body: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    let frame = build_frame(msg_id, phone, serial, body)?;
    stream.write_all(&frame).await?;
    let mut buffer = [0u8; 2048];
    let read = tokio::time::timeout(Duration::from_secs(4), stream.read(&mut buffer)).await??;
    let response = parse_frame(&buffer[..read])?;
    let result = response.body.get(2).copied().unwrap_or(255);
    println!(
        "  {} -> msg=0x{:04x} serial={} result={}",
        if result == 0 { "OK" } else { "NG" },
        response.msg_id,
        response.serial,
        result
    );
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let address = std::env::var("TSP_JT808_ADDR").unwrap_or_else(|_| "127.0.0.1:18080".to_string());
    let phone = std::env::var("TSP_JT808_PHONE").unwrap_or_else(|_| "013800000001".to_string());
    let auth = std::env::var("TSP_JT808_AUTH").unwrap_or_else(|_| "XT000001".to_string());

    println!("JT/T 808 simulator connecting to {address} as {phone}");
    let mut stream = TcpStream::connect(&address).await?;
    stream.set_nodelay(true)?;

    println!("[1] terminal authentication");
    exchange(&mut stream, MSG_TERMINAL_AUTH, &phone, 1, auth.as_bytes()).await?;

    for step in 0..5u32 {
        println!("[2] location report #{}", step + 1);
        exchange(
            &mut stream,
            MSG_LOCATION_REPORT,
            &phone,
            (step + 2) as u16,
            &location_body(step),
        )
        .await?;
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    println!("simulator finished; check the live console for vehicle 沪A·D8F21");
    Ok(())
}
