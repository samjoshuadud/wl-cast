//! Screen capture, H.264 encoding, streaming and input for Wayland compositors.
//!
//! - [`Capture`] grabs frames from a monitor.
//! - [`Encoder`] turns frames into H.264, on the GPU when possible.
//! - [`Input`] moves the mouse and presses keys. [`PressState`] says whether a button or key goes down or up.
//! - [`RtpSender`] streams H.264 over RTP.
//! - [`AccessToken`] is a random secret for viewer login.
//! - `RemoteSession` streams to a browser over WebRTC and applies its input (feature `webrtc`).
//!
//! Cargo features: `ffmpeg` for GPU encoding with VA-API or NVENC, `webrtc`, and `portal` for GNOME and KDE.
//!
//! ```no_run
//! # fn main() -> Result<(), wl_cast::Error> {
//! let mut capture = wl_cast::Capture::new(0)?;
//! let (width, height) = capture.size();
//! let mut encoder = wl_cast::Encoder::new(width, height)?;
//!
//! let frame = capture.next_frame()?;
//! let h264 = encoder.encode(&frame)?;
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs, missing_debug_implementations)]

mod auth;
mod capture;
mod encode;
mod error;
mod frame;
mod input;
mod stream;
#[cfg(feature = "webrtc")]
mod webrtc;

pub use auth::AccessToken;
pub use capture::{Capture, StopHandle};
pub use encode::{Encoder, EncoderConfig, H264Profile};
pub use error::Error;
pub use frame::Frame;
pub use input::{Input, MouseButton, PressState};
pub use stream::RtpSender;
#[cfg(feature = "webrtc")]
pub use webrtc::{RemoteSession, SessionConfig, SessionEvent};
