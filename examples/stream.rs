use wl_cast::{Capture, Encoder, RtpSender};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let destination = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:5004".to_string());

    let mut capture = Capture::new(0)?;
    let (width, height) = capture.size();
    let mut encoder = Encoder::new(width, height)?;
    println!("encoder: {}", encoder.name());
    let mut sender = RtpSender::new(destination.as_str())?;

    std::fs::create_dir_all("captures")?;
    std::fs::write("captures/stream.sdp", sender.sdp())?;
    println!("streaming to {destination}, press Ctrl+C to stop");
    println!(
        "watch it with: ffplay -protocol_whitelist file,udp,rtp -fflags nobuffer -flags low_delay captures/stream.sdp"
    );

    loop {
        let frame = capture.next_frame()?;
        let h264 = encoder.encode(&frame)?;
        sender.send(&h264, frame.captured_at)?;
    }
}
