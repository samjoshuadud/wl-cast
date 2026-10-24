#[cfg(feature = "ffmpeg")]
mod ffmpeg;
mod openh264;

use std::fmt;

use crate::{Error, Frame};

/// Encodes frames to H.264, using the GPU when possible.
///
/// With the `ffmpeg` feature it tries VA-API (Intel, AMD), then NVENC (Nvidia).
/// Otherwise, or if no GPU encoder works, it falls back to `OpenH264` in software.
pub struct Encoder {
    backend: Backend,
    width: u32,
    height: u32,
}

enum Backend {
    #[cfg(feature = "ffmpeg")]
    Hardware(ffmpeg::HardwareEncoder),
    Software(Box<openh264::SoftwareEncoder>),
}

impl Encoder {
    /// Opens an encoder for frames of exactly `width` by `height` pixels.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Encode`] if no encoder can be opened, not even `OpenH264`.
    pub fn new(width: u32, height: u32) -> Result<Self, Error> {
        #[cfg(feature = "ffmpeg")]
        if let Some(encoder) = ffmpeg::HardwareEncoder::open_any(width, height) {
            return Ok(Self {
                backend: Backend::Hardware(encoder),
                width,
                height,
            });
        }

        Ok(Self {
            backend: Backend::Software(Box::new(openh264::SoftwareEncoder::new()?)),
            width,
            height,
        })
    }

    /// Returns the name of the encoder in use, for example `h264_vaapi` or `openh264`.
    pub fn name(&self) -> &'static str {
        match &self.backend {
            #[cfg(feature = "ffmpeg")]
            Backend::Hardware(encoder) => encoder.name(),
            Backend::Software(_) => "openh264",
        }
    }

    /// Makes the next encoded frame a full picture, so a new viewer can start decoding.
    pub fn request_keyframe(&mut self) {
        match &mut self.backend {
            #[cfg(feature = "ffmpeg")]
            Backend::Hardware(encoder) => encoder.request_keyframe(),
            Backend::Software(encoder) => encoder.request_keyframe(),
        }
    }

    /// Returns the frame as H.264 bytes, ready to append to a `.h264` file or stream.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidFrame`] if the frame size does not match the encoder,
    /// or [`Error::Encode`] if the encoder fails.
    pub fn encode(&mut self, frame: &Frame) -> Result<Vec<u8>, Error> {
        let expected = (self.width as usize)
            .saturating_mul(self.height as usize)
            .saturating_mul(4);
        if frame.width != self.width || frame.height != self.height || frame.data.len() != expected
        {
            return Err(Error::InvalidFrame);
        }

        match &mut self.backend {
            #[cfg(feature = "ffmpeg")]
            Backend::Hardware(encoder) => encoder.encode(frame),
            Backend::Software(encoder) => encoder.encode(frame),
        }
    }
}

impl fmt::Debug for Encoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Encoder")
            .field("name", &self.name())
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}
