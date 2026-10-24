use std::time::Instant;

use wl_cast::{Encoder, Frame};

const IDR: u8 = 5;

fn frame(patch: u8) -> Frame {
    let mut data = vec![100; 64 * 64 * 4];
    data[..64 * 4 * 4].fill(patch);
    Frame {
        width: 64,
        height: 64,
        data,
        captured_at: Instant::now(),
    }
}

fn nal_types(h264: &[u8]) -> Vec<u8> {
    h264.windows(4)
        .filter(|w| w[..3] == [0, 0, 1])
        .map(|w| w[3] & 0x1F)
        .collect()
}

#[test]
fn requested_keyframe_is_an_idr_frame() {
    let mut encoder = Encoder::new(64, 64).unwrap();

    let first = encoder.encode(&frame(100)).unwrap();
    assert!(nal_types(&first).contains(&IDR));

    let second = encoder.encode(&frame(110)).unwrap();
    assert!(!nal_types(&second).contains(&IDR));

    encoder.request_keyframe();
    let third = encoder.encode(&frame(120)).unwrap();
    assert!(nal_types(&third).contains(&IDR));
}
