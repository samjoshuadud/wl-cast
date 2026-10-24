use openh264::{
    OpenH264API,
    encoder::{
        BitRate, Encoder as H264Encoder, EncoderConfig, FrameRate, IntraFramePeriod, UsageType,
    },
    formats::{BgraSliceU8, YUVBuffer},
};

use crate::{Error, Frame};

pub(crate) struct SoftwareEncoder {
    inner: H264Encoder,
}

impl SoftwareEncoder {
    pub(crate) fn new() -> Result<Self, Error> {
        let config = EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .max_frame_rate(FrameRate::from_hz(60.0))
            .bitrate(BitRate::from_bps(8_000_000))
            .adaptive_quantization(false)
            .background_detection(false)
            .intra_frame_period(IntraFramePeriod::from_num_frames(60));
        let inner = H264Encoder::with_api_config(OpenH264API::from_source(), config)
            .map_err(|e| Error::Encode(e.to_string()))?;
        Ok(Self { inner })
    }

    pub(crate) fn request_keyframe(&mut self) {
        self.inner.force_intra_frame();
    }

    pub(crate) fn encode(&mut self, frame: &Frame) -> Result<Vec<u8>, Error> {
        let size = (frame.width as usize, frame.height as usize);
        let yuv = YUVBuffer::from_bgra8_source(BgraSliceU8::new(&frame.data, size));
        let bitstream = self
            .inner
            .encode(&yuv)
            .map_err(|e| Error::Encode(e.to_string()))?;
        Ok(bitstream.to_vec())
    }
}
