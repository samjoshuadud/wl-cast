use std::{env, process::Command, thread, time::Duration};

use wl_cast::{Capture, Input, MouseButton};

const KEY_A: u32 = 30;

#[test]
#[ignore = "needs a Wayland compositor"]
fn captures_a_frame_and_sends_input() {
    for backend in ["ext", "wlr"] {
        // SAFETY: this test is one thread and does not spawn another before reading the variable.
        unsafe { env::set_var("WL_CAST_CAPTURE", backend) };
        let mut capture = Capture::new(0).unwrap();
        let (width, height) = capture.size();
        assert!(width > 0 && height > 0, "{backend}");

        let frame = capture.next_frame().unwrap();
        assert_eq!((frame.width, frame.height), (width, height), "{backend}");
        assert_eq!(
            frame.data.len(),
            width as usize * height as usize * 4,
            "{backend}"
        );
    }

    let mut input = Input::new(0).unwrap();
    input.move_to(0.5, 0.5).unwrap();
    input.button(MouseButton::Left, true).unwrap();
    input.button(MouseButton::Left, false).unwrap();
    input.scroll(0.0, -1.0).unwrap();
    input.key(KEY_A, true).unwrap();
    input.key(KEY_A, false).unwrap();
}

#[test]
#[ignore = "needs a Wayland compositor"]
fn keeps_capturing_after_the_output_is_resized() {
    let output = headless_output();
    let modes = ["640x480", "800x600"];
    for (backend, mode) in ["ext", "wlr"].into_iter().zip(modes) {
        // SAFETY: this test is one thread and does not spawn another before reading the variable.
        unsafe { env::set_var("WL_CAST_CAPTURE", backend) };
        let mut capture = Capture::new(0).unwrap();
        let first = capture.next_frame().unwrap();
        set_mode(&output, mode);
        let mut latest = first.clone();
        for _ in 0..20 {
            latest = capture.next_frame().unwrap();
            if (latest.width, latest.height) != (first.width, first.height) {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        assert_ne!(
            (latest.width, latest.height),
            (first.width, first.height),
            "{backend}"
        );
        assert_eq!(
            latest.data.len(),
            latest.width as usize * latest.height as usize * 4,
            "{backend}"
        );
    }
}

fn headless_output() -> String {
    let text = swaymsg(&["-t", "get_outputs"]);
    let start = text
        .find("HEADLESS")
        .expect("refusing to resize a physical monitor");
    let end = text[start..]
        .find('"')
        .map(|end| start + end)
        .expect("output name");
    text[start..end].to_string()
}

fn set_mode(output: &str, mode: &str) {
    swaymsg(&["output", output, "mode", mode]);
}

fn swaymsg(args: &[&str]) -> String {
    let output = Command::new("swaymsg").args(args).output().unwrap();
    assert!(
        output.status.success(),
        "swaymsg {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
