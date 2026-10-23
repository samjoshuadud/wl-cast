use std::{f64::consts::TAU, thread, time::Duration};

use wl_cast::Input;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = Input::new(0)?;

    println!("drawing a circle with the mouse on the first monitor...");
    for step in 0..=120 {
        let angle = step as f64 / 120.0 * TAU;
        input.move_to(0.5 + 0.2 * angle.cos(), 0.5 + 0.3 * angle.sin())?;
        thread::sleep(Duration::from_millis(16));
    }

    Ok(())
}
