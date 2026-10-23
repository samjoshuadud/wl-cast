#[cfg(feature = "ffmpeg")]
mod ffmpeg;
mod openh264;

use crate::{Error, Frame};

/// Encodes frames to H.264, using the GPU when possible.
///
/// With the `ffmpeg` feature it tries VA-API (Intel, AMD), then NVENC (Nvidia).
/// Otherwise, or if no GPU encoder works, it falls back to OpenH264 in software.
pub struct Encoder {
    backend: Backend,
}

enum Backend {
    #[cfg(feature = "ffmpeg")]
    Hardware(ffmpeg::HardwareEncoder),
    Software(Box<openh264::SoftwareEncoder>),
}

impl Encoder {
    pub fn new(width: u32, height: u32) -> Result<Self, Error> {
        #[cfg(feature = "ffmpeg")]
        if let Some(encoder) = ffmpeg::HardwareEncoder::open_any(width, height) {
            return Ok(Self {
                backend: Backend::Hardware(encoder),
            });
        }

        let _ = (width, height);
        Ok(Self {
            backend: Backend::Software(Box::new(openh264::SoftwareEncoder::new()?)),
        })
    }

    /// Which encoder is in use, for example `h264_vaapi` or `openh264`.
    pub fn name(&self) -> &'static str {
        match &self.backend {
            #[cfg(feature = "ffmpeg")]
            Backend::Hardware(encoder) => encoder.name(),
            Backend::Software(_) => "openh264",
        }
    }

    /// Returns the frame as H.264 bytes, ready to append to a `.h264` file or stream.
    pub fn encode(&mut self, frame: &Frame) -> Result<Vec<u8>, Error> {
        match &mut self.backend {
            #[cfg(feature = "ffmpeg")]
            Backend::Hardware(encoder) => encoder.encode(frame),
            Backend::Software(encoder) => encoder.encode(frame),
        }
    }
}
