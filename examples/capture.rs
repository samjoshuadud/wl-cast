use std::time::Instant;

use wl_cast::Capture;

const FRAMES: u32 = 100;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut capture = Capture::new(0)?;

    println!("capturing {FRAMES} frames, move the mouse or play a video...");
    let start = Instant::now();

    let mut frame = capture.next_frame()?;
    for _ in 1..FRAMES {
        frame = capture.next_frame()?;
    }

    let seconds = start.elapsed().as_secs_f64();
    println!(
        "{FRAMES} frames in {seconds:.2}s = {:.1} fps",
        FRAMES as f64 / seconds
    );

    let mut pixels = frame.data;
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }

    let path = "captures/frame.png";
    std::fs::create_dir_all("captures")?;
    image::RgbaImage::from_raw(frame.width, frame.height, pixels)
        .ok_or("buffer size mismatch")?
        .save(path)?;
    println!("saved {path} ({}x{})", frame.width, frame.height);

    Ok(())
}
