//! Deterministic robustness checks: malformed protocol bytes must return errors,
//! never panic.

use std::panic::{catch_unwind, AssertUnwindSafe};

use xingtu_tsp::{
    jt808::codec::{
        parse_frame, parse_location, parse_location_batch_typed, parse_registration,
        parse_terminal_attributes,
    },
    standards::{
        gb32960::{Gb32960Activation, Gb32960Frame, Gb32960Signature},
        jt1078::{Jt1078AacFrame, Jt1078H264Frame, Jt1078Packet},
        jt809::{Jt809Frame, Jt809Frame2019, Jt809Header2019},
    },
};

fn bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed;
    (0..len)
        .map(|_| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            (state >> 33) as u8
        })
        .collect()
}

#[test]
fn malformed_frames_never_panic() {
    for seed in 1..=1000u64 {
        let sample = bytes(seed, (seed as usize * 13) % 256);
        type Check<'a> = (&'static str, Box<dyn Fn() + 'a>);
        let checks: [Check<'_>; 7] = [
            (
                "jt808",
                Box::new(|| {
                    let _ = parse_frame(&sample);
                }),
            ),
            (
                "jt808-bodies",
                Box::new(|| {
                    let _ = parse_registration(&sample);
                    let _ = parse_terminal_attributes(&sample);
                    let _ = parse_location(&sample);
                    let _ = parse_location_batch_typed(&sample);
                }),
            ),
            (
                "jt809",
                Box::new(|| {
                    let _ = Jt809Frame::decode(&sample);
                }),
            ),
            (
                "jt809-2019",
                Box::new(|| {
                    let _ = Jt809Header2019::decode(&sample);
                    let _ = Jt809Frame2019::decode(&sample);
                }),
            ),
            (
                "gb32960",
                Box::new(|| {
                    let _ = Gb32960Frame::decode(&sample);
                    let _ = Gb32960Signature::decode(&sample);
                    let _ = Gb32960Activation::decode(&sample);
                }),
            ),
            (
                "jt1078-media",
                Box::new(|| {
                    let _ = Jt1078Packet::decode(&sample);
                }),
            ),
            (
                "jt1078-h264",
                Box::new(|| {
                    let _ = Jt1078H264Frame::parse(&sample);
                    let _ = Jt1078AacFrame::parse(&sample);
                }),
            ),
        ];
        for (name, check) in checks {
            let result = catch_unwind(AssertUnwindSafe(check));
            assert!(result.is_ok(), "{name} panicked for seed={seed}");
        }
    }
}
