use std::time::Instant;

use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, delegate_noop,
    protocol::{
        wl_output::WlOutput,
        wl_pointer::{Axis, ButtonState},
        wl_registry::{self, WlRegistry},
    },
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

use crate::Error;

const EXTENT: u32 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
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

/// Moves the mouse, clicks and scrolls on one output (monitor).
pub struct Input {
    conn: Connection,
    _queue: EventQueue<State>,
    pointer: ZwlrVirtualPointerV1,
    start: Instant,
}

impl Input {
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

        Ok(Self {
            conn,
            _queue: queue,
            pointer,
            start: Instant::now(),
        })
    }

    /// Moves the pointer to `x`, `y`, given as fractions of the screen from 0.0 to 1.0.
    pub fn move_to(&mut self, x: f64, y: f64) -> Result<(), Error> {
        let x = (x.clamp(0.0, 1.0) * EXTENT as f64) as u32;
        let y = (y.clamp(0.0, 1.0) * EXTENT as f64) as u32;
        self.pointer
            .motion_absolute(self.time(), x, y, EXTENT, EXTENT);
        self.pointer.frame();
        self.flush()
    }

    pub fn button(&mut self, button: MouseButton, pressed: bool) -> Result<(), Error> {
        let state = if pressed {
            ButtonState::Pressed
        } else {
            ButtonState::Released
        };
        self.pointer.button(self.time(), button.linux_code(), state);
        self.pointer.frame();
        self.flush()
    }

    /// Scrolls by `dx`, `dy` in pixels. Positive `dy` scrolls down.
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

    fn time(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    fn flush(&self) -> Result<(), Error> {
        self.conn.flush().map_err(|e| Error::Dispatch(e.into()))
    }
}

#[derive(Default)]
struct State {
    outputs: Vec<WlOutput>,
    pointer_manager: Option<ZwlrVirtualPointerManagerV1>,
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
                _ => {}
            }
        }
    }
}

delegate_noop!(State: ignore WlOutput);
delegate_noop!(State: ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ZwlrVirtualPointerV1);
