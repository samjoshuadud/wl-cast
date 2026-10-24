use openh264::{
    OpenH264API,
    encoder::{
        BitRate, Encoder as H264Encoder, EncoderConfig as OpenH264Config, FrameRate,
        IntraFramePeriod, Profile, UsageType,
    },
    formats::{BgraSliceU8, YUVBuffer},
};

use super::{EncoderConfig, H264Profile};
use crate::{Error, Frame};

pub(crate) struct SoftwareEncoder {
    inner: H264Encoder,
}

impl SoftwareEncoder {
    pub(crate) fn new(config: EncoderConfig) -> Result<Self, Error> {
        let mut encoder = OpenH264Config::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .max_frame_rate(FrameRate::from_hz(config.frame_rate as f32))
            .bitrate(BitRate::from_bps(config.bitrate_bps))
            .adaptive_quantization(false)
            .background_detection(false)
            .intra_frame_period(IntraFramePeriod::from_num_frames(config.frame_rate));
        if let Some(profile) = openh264_profile(config.profile) {
            encoder = encoder.profile(profile);
        }
        let inner = H264Encoder::with_api_config(OpenH264API::from_source(), encoder)
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

fn openh264_profile(profile: H264Profile) -> Option<Profile> {
    match profile {
        H264Profile::Unspecified => None,
        H264Profile::ConstrainedBaseline | H264Profile::Baseline => Some(Profile::Baseline),
        H264Profile::Main => Some(Profile::Main),
        H264Profile::High => Some(Profile::High),
    }
}
