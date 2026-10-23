use openh264::{
    OpenH264API,
    encoder::{BitRate, Encoder as H264Encoder, EncoderConfig, FrameRate, UsageType},
    formats::{BgraSliceU8, YUVBuffer},
};

use crate::{Error, Frame};

/// Encodes frames to H.264 in software.
pub struct Encoder {
    inner: H264Encoder,
}

impl Encoder {
    pub fn new() -> Result<Self, Error> {
        let config = EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .max_frame_rate(FrameRate::from_hz(60.0))
            .bitrate(BitRate::from_bps(8_000_000))
            .adaptive_quantization(false)
            .background_detection(false);
        let inner = H264Encoder::with_api_config(OpenH264API::from_source(), config)
            .map_err(|e| Error::Encode(e.to_string()))?;
        Ok(Self { inner })
    }

    /// Returns the frame as H.264 bytes, ready to append to a `.h264` file or stream.
    pub fn encode(&mut self, frame: &Frame) -> Result<Vec<u8>, Error> {
        let size = (frame.width as usize, frame.height as usize);
        let yuv = YUVBuffer::from_rgb_source(BgraSliceU8::new(&frame.data, size));
        let bitstream = self
            .inner
            .encode(&yuv)
            .map_err(|e| Error::Encode(e.to_string()))?;
        Ok(bitstream.to_vec())
    }
}
