use std::{thread, time::Duration};

use wl_cast::{Input, PressState};

const KEY_LEFTSHIFT: u32 = 42;
const KEY_H: u32 = 35;
const KEY_E: u32 = 18;
const KEY_L: u32 = 38;
const KEY_O: u32 = 24;
const KEY_1: u32 = 2;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = Input::new(0)?;

    println!("click into a text editor, typing in 3 seconds...");
    thread::sleep(Duration::from_secs(3));

    input.key(KEY_LEFTSHIFT, PressState::Pressed)?;
    tap(&mut input, KEY_H)?;
    input.key(KEY_LEFTSHIFT, PressState::Released)?;

    for key in [KEY_E, KEY_L, KEY_L, KEY_O] {
        tap(&mut input, key)?;
    }

    input.key(KEY_LEFTSHIFT, PressState::Pressed)?;
    tap(&mut input, KEY_1)?;
    input.key(KEY_LEFTSHIFT, PressState::Released)?;

    Ok(())
}

fn tap(input: &mut Input, key: u32) -> Result<(), wl_cast::Error> {
    input.key(key, PressState::Pressed)?;
    thread::sleep(Duration::from_millis(20));
    input.key(key, PressState::Released)?;
    thread::sleep(Duration::from_millis(20));
    Ok(())
}
