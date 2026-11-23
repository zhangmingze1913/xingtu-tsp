//! Deterministic robustness checks: malformed protocol bytes must return errors,
//! never panic.

use std::panic::{catch_unwind, AssertUnwindSafe};

use xingtu_tsp::{
    jt808::codec::parse_frame,
    standards::{
        gb32960::Gb32960Frame,
        jt1078::{Jt1078AacFrame, Jt1078H264Frame, Jt1078Packet},
        jt809::Jt809Frame,
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
    for seed in 1..=200u64 {
        let sample = bytes(seed, (seed as usize * 13) % 256);
        type Check<'a> = (&'static str, Box<dyn Fn() + 'a>);
        let checks: [Check<'_>; 5] = [
            (
                "jt808",
                Box::new(|| {
                    let _ = parse_frame(&sample);
                }),
            ),
            (
                "jt809",
                Box::new(|| {
                    let _ = Jt809Frame::decode(&sample);
                }),
            ),
            (
                "gb32960",
                Box::new(|| {
                    let _ = Gb32960Frame::decode(&sample);
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
