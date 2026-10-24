use std::{
    fmt,
    os::fd::{AsFd, OwnedFd},
    time::Instant,
};

use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, WEnum, delegate_noop,
    protocol::{
        wl_keyboard::{self, KeymapFormat, WlKeyboard},
        wl_output::WlOutput,
        wl_pointer::{Axis, ButtonState},
        wl_registry::{self, WlRegistry},
        wl_seat::WlSeat,
    },
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

use crate::Error;

const EXTENT: u32 = 10_000;
const KEY_CAPSLOCK: u32 = 58;
const LOCK: u32 = 1 << 1;

/// Whether a button or key is going down or coming up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PressState {
    /// The button or key is going down.
    Pressed,
    /// The button or key is coming up.
    Released,
}

/// A mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    /// The left button.
    Left,
    /// The right button.
    Right,
    /// The middle button, usually the wheel.
    Middle,
}

impl MouseButton {
    fn linux_code(self) -> u32 {
        match self {
            MouseButton::Left => 0x110,
            MouseButton::Right => 0x111,
            MouseButton::Middle => 0x112,
        }
    }
}

/// Controls the mouse and keyboard through virtual input devices.
///
/// The mouse covers one monitor.
pub struct Input {
    conn: Connection,
    _queue: EventQueue<State>,
    pointer: ZwlrVirtualPointerV1,
    keyboard: ZwpVirtualKeyboardV1,
    held_modifiers: Vec<u32>,
    locked_modifiers: u32,
    start: Instant,
}

impl Input {
    /// Creates a virtual mouse and keyboard.
    ///
    /// The mouse covers the monitor at `output_index`. The keyboard uses the compositor's current keymap.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] if the compositor lacks the virtual pointer or keyboard protocols,
    /// or [`Error::OutputNotFound`] if there is no monitor at `output_index`.
    pub fn new(output_index: usize) -> Result<Self, Error> {
        let conn = Connection::connect_to_env()?;
        let mut queue = conn.new_event_queue::<State>();
        let qh = queue.handle();
        let mut state = State::default();

        conn.display().get_registry(&qh, ());
        queue.roundtrip(&mut state)?;

        let output = state
            .outputs
            .get(output_index)
            .ok_or(Error::OutputNotFound(output_index))?;
        let manager = state.pointer_manager.as_ref().ok_or(Error::Unsupported(
            "zwlr_virtual_pointer_manager_v1 version 2",
        ))?;
        let pointer = manager.create_virtual_pointer_with_output(None, Some(output), &qh, ());

        let seat = state.seat.clone().ok_or(Error::Unsupported("wl_seat"))?;
        seat.get_keyboard(&qh, ());
        queue.roundtrip(&mut state)?;

        let (keymap, keymap_size) = state
            .keymap
            .take()
            .ok_or(Error::Unsupported("XKB keymap"))?;
        let keyboard_manager = state
            .keyboard_manager
            .as_ref()
            .ok_or(Error::Unsupported("zwp_virtual_keyboard_manager_v1"))?;
        let keyboard = keyboard_manager.create_virtual_keyboard(&seat, &qh, ());
        keyboard.keymap(KeymapFormat::XkbV1.into(), keymap.as_fd(), keymap_size);

        Ok(Self {
            conn,
            _queue: queue,
            pointer,
            keyboard,
            held_modifiers: Vec::new(),
            locked_modifiers: 0,
            start: Instant::now(),
        })
    }

    /// Moves the pointer to `x`, `y`, given as fractions of the screen from 0.0 to 1.0.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Dispatch`] if the connection to the compositor is lost.
    pub fn move_to(&mut self, x: f64, y: f64) -> Result<(), Error> {
        let x = (x.clamp(0.0, 1.0) * EXTENT as f64) as u32;
        let y = (y.clamp(0.0, 1.0) * EXTENT as f64) as u32;
        self.pointer
            .motion_absolute(self.time(), x, y, EXTENT, EXTENT);
        self.pointer.frame();
        self.flush()
    }

    /// Presses or releases a mouse button.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Dispatch`] if the connection to the compositor is lost.
    pub fn button(&mut self, button: MouseButton, press: PressState) -> Result<(), Error> {
        let state = match press {
            PressState::Pressed => ButtonState::Pressed,
            PressState::Released => ButtonState::Released,
        };
        self.pointer.button(self.time(), button.linux_code(), state);
        self.pointer.frame();
        self.flush()
    }

    /// Scrolls by `dx`, `dy` in pixels. Positive `dy` scrolls down.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Dispatch`] if the connection to the compositor is lost.
    pub fn scroll(&mut self, dx: f64, dy: f64) -> Result<(), Error> {
        let time = self.time();
        if dx != 0.0 {
            self.pointer.axis(time, Axis::HorizontalScroll, dx);
        }
        if dy != 0.0 {
            self.pointer.axis(time, Axis::VerticalScroll, dy);
        }
        self.pointer.frame();
        self.flush()
    }

    /// Presses or releases a key.
    ///
    /// `code` is a Linux key code, for example 30 for A. The list is in `linux/input-event-codes.h`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Dispatch`] if the connection to the compositor is lost.
    pub fn key(&mut self, code: u32, press: PressState) -> Result<(), Error> {
        let state = match press {
            PressState::Pressed => wl_keyboard::KeyState::Pressed,
            PressState::Released => wl_keyboard::KeyState::Released,
        };
        self.keyboard.key(self.time(), code, state.into());

        let down = press == PressState::Pressed;
        if code == KEY_CAPSLOCK && down {
            self.locked_modifiers ^= LOCK;
        } else if modifier_mask(code).is_some() {
            self.held_modifiers.retain(|&held| held != code);
            if down {
                self.held_modifiers.push(code);
            }
        }
        let depressed = self
            .held_modifiers
            .iter()
            .filter_map(|&held| modifier_mask(held))
            .fold(0, |mask, bit| mask | bit);
        self.keyboard
            .modifiers(depressed, 0, self.locked_modifiers, 0);

        self.flush()
    }

    fn time(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    fn flush(&self) -> Result<(), Error> {
        self.conn
            .flush()
            .map_err(|error| Error::Dispatch(error.to_string()))
    }
}

impl fmt::Debug for Input {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Input")
            .field("held_modifiers", &self.held_modifiers)
            .field("locked_modifiers", &self.locked_modifiers)
            .finish_non_exhaustive()
    }
}

fn modifier_mask(code: u32) -> Option<u32> {
    match code {
        42 | 54 => Some(1 << 0),
        29 | 97 => Some(1 << 2),
        56 => Some(1 << 3),
        125 | 126 => Some(1 << 6),
        100 => Some(1 << 7),
        _ => None,
    }
}

#[derive(Default)]
struct State {
    outputs: Vec<WlOutput>,
    pointer_manager: Option<ZwlrVirtualPointerManagerV1>,
    keyboard_manager: Option<ZwpVirtualKeyboardManagerV1>,
    seat: Option<WlSeat>,
    keymap: Option<(OwnedFd, u32)>,
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_output" => state.outputs.push(registry.bind(name, 1, qh, ())),
                "zwlr_virtual_pointer_manager_v1" if version >= 2 => {
                    state.pointer_manager = Some(registry.bind(name, 2, qh, ()))
                }
                "zwp_virtual_keyboard_manager_v1" => {
                    state.keyboard_manager = Some(registry.bind(name, 1, qh, ()))
                }
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind(name, 1, qh, ()))
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _keyboard: &WlKeyboard,
        event: wl_keyboard::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Keymap {
            format: WEnum::Value(KeymapFormat::XkbV1),
            fd,
            size,
        } = event
        {
            state.keymap = Some((fd, size));
        }
    }
}

delegate_noop!(State: ignore WlOutput);
delegate_noop!(State: ignore WlSeat);
delegate_noop!(State: ZwpVirtualKeyboardManagerV1);
delegate_noop!(State: ZwpVirtualKeyboardV1);
delegate_noop!(State: ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ZwlrVirtualPointerV1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_keys_map_to_xkb_bits() {
        assert_eq!(modifier_mask(42), Some(1));
        assert_eq!(modifier_mask(54), Some(1));
        assert_eq!(modifier_mask(29), Some(4));
        assert_eq!(modifier_mask(56), Some(8));
        assert_eq!(modifier_mask(125), Some(64));
        assert_eq!(modifier_mask(30), None);
    }
}
