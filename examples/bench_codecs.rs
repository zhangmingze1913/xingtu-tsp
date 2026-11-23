//! Minimal codec throughput baseline. Not a production load test.
//!
//! Run with:
//! `cargo run --release --example bench_codecs`

use std::time::Instant;

use xingtu_tsp::standards::gb32960::{Gb32960Frame, GB32960_REALTIME};
use xingtu_tsp::standards::jt809::{Jt809Frame2019, Jt809Header2019};

fn rate(name: &str, iterations: u64, elapsed: std::time::Duration) {
    let secs = elapsed.as_secs_f64();
    println!(
        "{name:<28} {iterations:>9} ops in {secs:>8.3}s  {:>12.0} ops/s",
        iterations as f64 / secs
    );
}

fn main() {
    let gb = Gb32960Frame {
        command: GB32960_REALTIME,
        response: 0xFE,
        vin: "LSVAA123456789012".to_string(),
        encrypt: 1,
        data: vec![0u8; 64],
    };
    let gb_raw = gb.encode().expect("gb encode");
    let iterations = 200_000u64;
    let started = Instant::now();
    for _ in 0..iterations {
        let _ = gb.encode().expect("gb encode");
    }
    rate("32960 encode", iterations, started.elapsed());

    let started = Instant::now();
    for _ in 0..iterations {
        let _ = Gb32960Frame::decode(&gb_raw).expect("gb decode");
    }
    rate("32960 decode", iterations, started.elapsed());

    let jt = Jt809Frame2019 {
        msg_sn: 1,
        msg_id: 0x1200,
        center_id: 1,
        version: [1, 2, 15],
        encrypt: 0,
        encrypt_key: 0,
        time: 1_700_000_000,
        body: vec![0x5b; 256],
    };
    let jt_raw = jt.encode().expect("jt encode");
    let iterations = 100_000u64;
    let started = Instant::now();
    for _ in 0..iterations {
        let _ = jt.encode().expect("jt encode");
    }
    rate("809 encode (256B body)", iterations, started.elapsed());

    let started = Instant::now();
    for _ in 0..iterations {
        let _ = Jt809Frame2019::decode(&jt_raw).expect("jt decode");
    }
    rate("809 decode (256B body)", iterations, started.elapsed());

    println!(
        "809 header size = {}, escaped frame size = {}",
        Jt809Header2019::LEN,
        jt_raw.len()
    );
}
