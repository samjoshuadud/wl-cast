use std::{fs::File, io::Write, time::Instant};

use wl_cast::{Capture, Encoder};

const FRAMES: u32 = 300;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut capture = Capture::new(0)?;
    let (width, height) = capture.size();
    let mut encoder = Encoder::new(width, height)?;
    println!("encoder: {}", encoder.name());

    std::fs::create_dir_all("captures")?;
    let path = "captures/out.h264";
    let mut file = File::create(path)?;

    println!("recording {FRAMES} frames, play a video on the first monitor...");
    let start = Instant::now();

    for _ in 0..FRAMES {
        let frame = capture.next_frame()?;
        let h264 = encoder.encode(&frame)?;
        file.write_all(&h264)?;
    }

    let seconds = start.elapsed().as_secs_f64();
    println!(
        "{FRAMES} frames in {seconds:.2}s = {:.1} fps",
        FRAMES as f64 / seconds
    );
    println!("saved {path}, play it with: ffplay -framerate 60 {path}");

    Ok(())
}
