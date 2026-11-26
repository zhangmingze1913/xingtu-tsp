//! Standards conformance negative/round-trip baseline.
//!
//! These are protocol-shape tests, not certification. They make the current
//! codec seams fail loudly instead of silently accepting corrupt frames.

use xingtu_tsp::jt808::codec;
use xingtu_tsp::standards::{gb32960, jt809};

#[test]
fn jt809_login_roundtrip_and_bad_crc() {
    let login = jt809::Jt809Login {
        user_id: 1,
        password: "secret".to_string(),
        center_id: 7,
        down_link_ip: "127.0.0.1".to_string(),
        down_link_port: 9001,
    };
    let body = login.encode().expect("login body");
    let frame = jt809::Jt809Frame {
        msg_id: 0x1001,
        msg_sn: 42,
        encrypt: 0,
        compress: 0,
        body,
    };
    let raw = frame.encode();
    let decoded = jt809::Jt809Frame::decode(&raw).expect("decode 809");
    assert_eq!(decoded.msg_sn, 42);
    assert_eq!(decoded.msg_id, 0x1001);
    assert_eq!(
        jt809::Jt809Login::decode(&decoded.body).expect("login"),
        login
    );

    let mut corrupt = raw.clone();
    let crc_index = corrupt.len().saturating_sub(5);
    corrupt[crc_index] ^= 0xff;
    assert!(jt809::Jt809Frame::decode(&corrupt).is_err());
}

#[test]
fn gb32960_header_roundtrip_and_bad_vin() {
    let frame = gb32960::Gb32960Frame {
        command: gb32960::GB32960_REALTIME,
        response: 0x01,
        vin: "LSVAA123456789012".to_string(),
        encrypt: 0,
        data: vec![0u8; 20],
    };
    let raw = frame.encode().expect("encode 32960");
    let decoded = gb32960::Gb32960Frame::decode(&raw).expect("decode 32960");
    assert_eq!(decoded.vin, frame.vin);
    assert_eq!(decoded.command, gb32960::GB32960_REALTIME);

    let mut bad = raw.clone();
    bad[4] = b'?';
    assert!(gb32960::Gb32960Frame::decode(&bad).is_err());

    let mut short = raw.clone();
    short.pop();
    short.pop();
    assert!(gb32960::Gb32960Frame::decode(&short).is_err());
}

fn decode_hex(value: &str) -> Vec<u8> {
    let value: String = value.chars().filter(|ch| !ch.is_whitespace()).collect();
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).expect("hex"))
        .collect()
}

#[test]
fn static_frame_fixtures_decode() {
    let jt809 = decode_hex(include_str!("fixtures/standards/jt809_login.hex"));
    let frame = jt809::Jt809Frame::decode(&jt809).expect("jt809 fixture");
    assert_eq!(frame.msg_id, 0x1001);
    assert_eq!(frame.msg_sn, 42);
    // Fixture predates the 2019 table-11 login body; header/CRC coverage remains.
    assert!(!frame.body.is_empty());

    let gb = decode_hex(include_str!("fixtures/standards/gb32960_realtime.hex"));
    let frame = gb32960::Gb32960Frame::decode(&gb).expect("gb fixture");
    assert_eq!(frame.command, gb32960::GB32960_REALTIME);
    assert_eq!(frame.vin, "LSVAA123456789012");
    assert_eq!(frame.data.len(), 20);
}

#[test]
fn jt808_body_fixtures_decode_fields() {
    let registration = decode_hex(include_str!("fixtures/standards/jt808_0100_2019.hex"));
    let parsed = codec::parse_registration(&registration).expect("registration fields");
    assert!(!parsed.manufacturer.is_empty());
    assert!(!parsed.model.is_empty());

    let attributes = decode_hex(include_str!("fixtures/standards/jt808_0107_2019.hex"));
    let parsed = codec::parse_terminal_attributes(&attributes).expect("attribute fields");
    assert!(!parsed.raw_hex.is_empty());

    let location = decode_hex(include_str!("fixtures/standards/jt808_0200_mandatory.hex"));
    let parsed = codec::parse_location(&location).expect("location fields");
    assert!(parsed.lat.abs() > 1.0 && parsed.lon.abs() > 1.0);

    let alarm_ack = decode_hex(include_str!("fixtures/standards/jt808_8203.hex"));
    assert!(alarm_ack.len() >= 2);
}

#[test]
fn jt808_truncated_bodies_are_rejected() {
    let location = decode_hex(include_str!("fixtures/standards/jt808_0200_mandatory.hex"));
    for cut in [1usize, 4, 10, 27] {
        assert!(
            codec::parse_location(&location[..cut]).is_err(),
            "cut={cut}"
        );
    }
}

#[test]
fn jt808_truncated_frames_are_rejected() {
    let raw = decode_hex(include_str!("fixtures/standards/jt808_0200_mandatory.hex"));
    for cut in [1usize, 10, raw.len() / 2, raw.len().saturating_sub(1)] {
        assert!(codec::parse_frame(&raw[..cut]).is_err(), "cut={cut}");
    }
}
