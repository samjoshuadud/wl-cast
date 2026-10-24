use std::env;

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
