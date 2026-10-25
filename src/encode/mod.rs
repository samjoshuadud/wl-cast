#[cfg(feature = "ffmpeg")]
mod ffmpeg;
mod openh264;

use std::fmt;

use crate::{Error, Frame};

const DEFAULT_BITRATE_BPS: u32 = 8_000_000;
const DEFAULT_FRAME_RATE: u32 = 60;

/// Bitrate, frame rate and H.264 profile for an [`Encoder`].
///
/// [`Encoder::new`] uses [`EncoderConfig::default`]: 8 Mbit/s, 60 frames per second,
/// and the encoder's own profile.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EncoderConfig {
    /// Target bitrate in bits per second.
    pub bitrate_bps: u32,
    /// Frames per second.
    pub frame_rate: u32,
    /// H.264 profile. [`H264Profile::Unspecified`] leaves the choice to the encoder.
    pub profile: H264Profile,
}

impl EncoderConfig {
    /// Creates encoder settings.
    ///
    /// `bitrate_bps` is the target rate in bits per second. `frame_rate` is frames per second.
    /// The profile stays [`H264Profile::Unspecified`].
    pub fn new(bitrate_bps: u32, frame_rate: u32) -> Self {
        Self {
            bitrate_bps,
            frame_rate,
            profile: H264Profile::Unspecified,
        }
    }
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self::new(DEFAULT_BITRATE_BPS, DEFAULT_FRAME_RATE)
    }
}

/// An H.264 profile.
///
/// Constrained Baseline is the profile most wireless display sinks accept.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum H264Profile {
    /// Let the encoder pick. This is the default.
    #[default]
    Unspecified,
    /// H.264 Constrained Baseline.
    ConstrainedBaseline,
    /// H.264 Baseline.
    Baseline,
    /// H.264 Main.
    Main,
    /// H.264 High.
    High,
}

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
    /// Uses [`EncoderConfig::default`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Encode`] if `width` or `height` is odd, or if no encoder can be opened,
    /// not even `OpenH264`.
    pub fn new(width: u32, height: u32) -> Result<Self, Error> {
        Self::with_config(width, height, EncoderConfig::default())
    }

    /// Opens an encoder with the given bitrate, frame rate and profile.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Encode`] if `width` or `height` is odd, the bitrate or frame rate is 0,
    /// or if no encoder can be opened.
    pub fn with_config(width: u32, height: u32, config: EncoderConfig) -> Result<Self, Error> {
        if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Err(Error::Encode("width and height must be even".into()));
        }
        if config.bitrate_bps == 0 {
            return Err(Error::Encode("bitrate must be at least 1".into()));
        }
        if config.frame_rate == 0 {
            return Err(Error::Encode("frame rate must be at least 1".into()));
        }

        #[cfg(feature = "ffmpeg")]
        if let Some(encoder) = ffmpeg::HardwareEncoder::open_any(width, height, config) {
            return Ok(Self {
                backend: Backend::Hardware(encoder),
                width,
                height,
            });
        }

        Ok(Self {
            backend: Backend::Software(Box::new(openh264::SoftwareEncoder::new(config)?)),
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

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use crate::Frame;

    use super::*;

    #[test]
    fn default_settings_keep_the_previous_bitrate_and_frame_rate() {
        let config = EncoderConfig::default();
        assert_eq!(config.bitrate_bps, 8_000_000);
        assert_eq!(config.frame_rate, 60);
        assert_eq!(config.profile, H264Profile::Unspecified);
        assert_eq!(EncoderConfig::new(8_000_000, 60), config);
    }

    #[test]
    fn rejects_an_odd_width_or_height() {
        assert!(matches!(
            Encoder::new(17, 16),
            Err(Error::Encode(reason)) if reason == "width and height must be even"
        ));
        assert!(matches!(
            Encoder::with_config(16, 17, EncoderConfig::default()),
            Err(Error::Encode(reason)) if reason == "width and height must be even"
        ));
    }

    #[test]
    fn rejects_a_zero_bitrate_or_frame_rate() {
        let zero_bitrate = EncoderConfig {
            bitrate_bps: 0,
            ..EncoderConfig::default()
        };
        assert!(Encoder::with_config(16, 16, zero_bitrate).is_err());
        let zero_rate = EncoderConfig {
            frame_rate: 0,
            ..EncoderConfig::default()
        };
        assert!(Encoder::with_config(16, 16, zero_rate).is_err());
    }

    #[test]
    fn encodes_with_a_lower_bitrate_and_baseline_profile() {
        let config = EncoderConfig {
            profile: H264Profile::Baseline,
            ..EncoderConfig::new(1_000_000, 30)
        };
        let mut encoder = Encoder::with_config(16, 16, config).unwrap();
        let frame = Frame::new(16, 16, vec![0; 16 * 16 * 4], Instant::now());
        let bytes = encoder.encode(&frame).unwrap();
        assert!(!bytes.is_empty());
    }
}
