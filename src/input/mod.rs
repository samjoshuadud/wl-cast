use std::{env, fmt};

#[cfg(feature = "portal")]
mod portal;
mod wlr;

use crate::Error;

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

/// Controls the mouse and keyboard.
///
/// The mouse covers one monitor. On a compositor with the virtual pointer and virtual keyboard,
/// those are used. With the `portal` feature, compositors without them, such as GNOME and KDE,
/// go through xdg-desktop-portal. The desktop then asks to allow remote control, and the monitor
/// index is not used.
pub struct Input {
    backend: Backend,
}

enum Backend {
    Wlr(wlr::WlrInput),
    #[cfg(feature = "portal")]
    Portal(portal::PortalInput),
}

impl Input {
    /// Creates a virtual mouse and keyboard.
    ///
    /// The mouse covers the monitor at `output_index`. The keyboard uses the compositor's current keymap.
    /// With the `portal` feature, a compositor without the virtual devices uses the desktop portal instead,
    /// shows a permission dialog, and ignores `output_index`.
    /// Set `WL_CAST_INPUT` to `wlr` or `portal` to force one.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] if no input method is available, [`Error::OutputNotFound`] if
    /// there is no monitor at `output_index` on the virtual-device path, [`Error::Connect`] if there
    /// is no compositor, or `Error::Portal` if the portal fails or the choice is cancelled.
    pub fn new(output_index: usize) -> Result<Self, Error> {
        let preference = env::var("WL_CAST_INPUT").ok();
        let backend = open(output_index, preference.as_deref())?;
        Ok(Self { backend })
    }

    /// Moves the pointer to `x`, `y`, given as fractions of the screen from 0.0 to 1.0.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Dispatch`] if the connection to the compositor is lost, or `Error::Portal`
    /// if the portal session fails.
    pub fn move_to(&mut self, x: f64, y: f64) -> Result<(), Error> {
        match &mut self.backend {
            Backend::Wlr(input) => input.move_to(x, y),
            #[cfg(feature = "portal")]
            Backend::Portal(input) => input.move_to(x, y),
        }
    }

    /// Presses or releases a mouse button.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Dispatch`] if the connection to the compositor is lost, or `Error::Portal`
    /// if the portal session fails.
    pub fn button(&mut self, button: MouseButton, press: PressState) -> Result<(), Error> {
        match &mut self.backend {
            Backend::Wlr(input) => input.button(button, press),
            #[cfg(feature = "portal")]
            Backend::Portal(input) => input.button(button, press),
        }
    }

    /// Scrolls by `dx`, `dy` in pixels. Positive `dy` scrolls down.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Dispatch`] if the connection to the compositor is lost, or `Error::Portal`
    /// if the portal session fails.
    pub fn scroll(&mut self, dx: f64, dy: f64) -> Result<(), Error> {
        match &mut self.backend {
            Backend::Wlr(input) => input.scroll(dx, dy),
            #[cfg(feature = "portal")]
            Backend::Portal(input) => input.scroll(dx, dy),
        }
    }

    /// Presses or releases a key.
    ///
    /// `code` is a Linux key code, for example 30 for A. The list is in `linux/input-event-codes.h`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Dispatch`] if the connection to the compositor is lost, or `Error::Portal`
    /// if the portal session fails.
    pub fn key(&mut self, code: u32, press: PressState) -> Result<(), Error> {
        match &mut self.backend {
            Backend::Wlr(input) => input.key(code, press),
            #[cfg(feature = "portal")]
            Backend::Portal(input) => input.key(code, press),
        }
    }
}

impl fmt::Debug for Input {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Input")
            .field("backend", &self.backend)
            .finish()
    }
}

impl fmt::Debug for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Backend::Wlr(input) => input.fmt(f),
            #[cfg(feature = "portal")]
            Backend::Portal(input) => input.fmt(f),
        }
    }
}

fn open(output_index: usize, preference: Option<&str>) -> Result<Backend, Error> {
    match preference {
        Some("portal") => match portal_backend() {
            Some(result) => result,
            None => Err(Error::Unsupported(
                "xdg-desktop-portal input without the portal feature",
            )),
        },
        Some("wlr") => Ok(Backend::Wlr(wlr::WlrInput::new(output_index)?)),
        _ => match wlr::WlrInput::new(output_index) {
            Ok(input) => Ok(Backend::Wlr(input)),
            Err(error) if portal_fallback(&error) => match portal_backend() {
                Some(result) => result,
                None => Err(error),
            },
            Err(error) => Err(error),
        },
    }
}

fn portal_fallback(error: &Error) -> bool {
    matches!(error, Error::Unsupported(_))
}

fn portal_backend() -> Option<Result<Backend, Error>> {
    #[cfg(feature = "portal")]
    {
        Some(portal::PortalInput::new().map(Backend::Portal))
    }
    #[cfg(not(feature = "portal"))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_missing_virtual_device_falls_back_to_the_portal() {
        assert!(portal_fallback(&Error::Unsupported(
            "zwlr_virtual_pointer_manager_v1 version 2"
        )));
        assert!(!portal_fallback(&Error::Connect("no compositor".into())));
        assert!(!portal_fallback(&Error::OutputNotFound(1)));
    }
}
