use std::time::Instant;

use wl_cast::{Encoder, Error, Frame};

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

fn raw_frame(width: u32, height: u32, len: usize) -> Frame {
    Frame {
        width,
        height,
        data: vec![100; len],
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

#[test]
fn rejects_a_frame_of_a_different_size() {
    let mut encoder = Encoder::new(64, 64).unwrap();
    let result = encoder.encode(&raw_frame(32, 32, 32 * 32 * 4));
    assert!(matches!(result, Err(Error::InvalidFrame)));
}

#[test]
fn rejects_a_frame_with_missing_data() {
    let mut encoder = Encoder::new(64, 64).unwrap();
    let result = encoder.encode(&raw_frame(64, 64, 100));
    assert!(matches!(result, Err(Error::InvalidFrame)));
}

#[test]
fn accepts_a_frame_of_the_right_size_after_a_rejected_one() {
    let mut encoder = Encoder::new(64, 64).unwrap();
    assert!(encoder.encode(&raw_frame(64, 64, 100)).is_err());
    assert!(encoder.encode(&frame(100)).is_ok());
}

#[test]
fn debug_output_shows_the_encoder_and_size() {
    let encoder = Encoder::new(64, 32).unwrap();
    let debug = format!("{encoder:?}");
    assert!(debug.contains(encoder.name()));
    assert!(debug.contains("width: 64"));
    assert!(debug.contains("height: 32"));
}

#[test]
fn encoder_can_move_between_threads() {
    fn assert_send<T: Send>() {}
    assert_send::<Encoder>();
}
