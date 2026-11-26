//! Simulated JT/T 1078 signaling + media pipeline.
//! This uses synthetic packets and local codecs; it is not device interop.

use xingtu_tsp::jt808::codec::{build_frame, parse_frame};
use xingtu_tsp::standards::jt1078::{
    Jt1078FrameAssembler, Jt1078H264Frame, Jt1078Packet, Jt1078RealtimeRequest,
    Jt1078StreamPipeline, MSG_REALTIME_REQUEST,
};

#[test]
fn simulated_9101_request_and_media_pipeline() {
    let request = Jt1078RealtimeRequest {
        server_ip: "127.0.0.1".to_string(),
        tcp_port: 1078,
        udp_port: 1079,
        channel: 1,
        data_type: 0,
        stream_type: 0,
    };
    let body = request.encode().expect("9101 body");
    let raw = build_frame(MSG_REALTIME_REQUEST, "013800000001", 7, &body).expect("808 frame");
    let frame = parse_frame(&raw).expect("parse 808 frame");
    assert_eq!(frame.msg_id, MSG_REALTIME_REQUEST);
    assert_eq!(frame.serial, 7);
    assert_eq!(Jt1078RealtimeRequest::decode(&frame.body).unwrap(), request);

    let terminal_ack = [0, 7, 0, 0x91, 0x01];
    let ack = build_frame(0x0001, "013800000001", 8, &terminal_ack).expect("ack frame");
    let parsed = parse_frame(&ack).expect("parse ack");
    assert_eq!(parsed.msg_id, 0x0001);
    assert_eq!(parsed.body, terminal_ack);

    let mut assembler = Jt1078FrameAssembler::new(4096);
    let h264 = [
        0, 0, 0, 1, 0x67, 0x42, 0x00, 0x1F, 0, 0, 0, 1, 0x68, 0xCE, 0x3C, 0x80, 0, 0, 1, 0x65,
        0x88, 0x84,
    ];
    let (first, second) = h264.split_at(10);
    let packet = |sequence: u16, marker: u8, data: &[u8]| Jt1078Packet {
        flags: 0x80,
        marker_payload: marker,
        sequence,
        sim: [0x89, 0x86, 0x00, 0x12, 0x34, 0x56],
        channel: 1,
        data_type: 0x01,
        timestamp: 1,
        last_i_frame_interval: 0,
        last_frame_interval: 0,
        body: data.to_vec(),
    };
    assert_eq!(assembler.push(&packet(1, 0x62, first)), None);
    let assembled = assembler
        .push(&packet(2, 0xE2, second))
        .expect("marker completes frame");
    assert_eq!(assembled, h264);

    let parsed = Jt1078H264Frame::parse(&assembled).expect("h264");
    assert_eq!(parsed.nals.len(), 3);
    assert!(parsed.is_keyframe());
}

fn media_packet(sequence: u16, marker: bool, body: u8) -> Jt1078Packet {
    Jt1078Packet {
        flags: 0x80,
        marker_payload: if marker { 0x80 } else { 0x00 },
        sequence,
        sim: [0x89, 0x86, 0x00, 0x12, 0x34, 0x56],
        channel: 1,
        data_type: 0,
        timestamp: 1,
        last_i_frame_interval: 0,
        last_frame_interval: 0,
        body: vec![body],
    }
}

#[test]
fn stream_pipeline_reorders_duplicates_and_skips_gaps() {
    let mut pipeline = Jt1078StreamPipeline::new(16, 16, 2);
    assert!(pipeline.push(media_packet(1, false, 1)).is_empty());
    assert!(pipeline.push(media_packet(3, true, 3)).is_empty());
    assert!(pipeline.push(media_packet(3, true, 3)).is_empty());
    let frames = pipeline.push(media_packet(2, false, 2));
    assert_eq!(frames, vec![vec![1, 2, 3]]);
    assert_eq!(pipeline.stats().frames, 1);
    assert_eq!(pipeline.stats().duplicates, 1);

    let mut gap = Jt1078StreamPipeline::new(16, 16, 1);
    assert!(gap.push(media_packet(5, false, 5)).is_empty());
    assert!(gap.push(media_packet(7, false, 7)).is_empty());
    let frames = gap.push(media_packet(8, true, 8));
    assert_eq!(frames, vec![vec![5, 7, 8]]);
    assert_eq!(gap.stats().gaps_skipped, 1);
}
