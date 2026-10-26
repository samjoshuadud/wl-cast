use std::ptr;

use ffmpeg_next::{
    self as ffmpeg, Dictionary, Packet, Rational, codec, color, encoder, ffi, format::Pixel, frame,
    picture,
};
use yuv::YuvBiPlanarImageMut;

use super::{EncoderConfig, H264Profile, convert};
use crate::{Error, Frame};

const X264: &str = "libx264";
const CANDIDATES: [&str; 3] = ["h264_vaapi", "h264_nvenc", X264];
const VBV_MILLISECONDS: u64 = 500;
const VERYFAST_MAX_PIXEL_RATE: u64 = 1920 * 1080 * 60;
const SUPERFAST_MAX_PIXEL_RATE: u64 = 2 * VERYFAST_MAX_PIXEL_RATE;

pub(crate) struct FfmpegEncoder {
    name: &'static str,
    encoder: encoder::Video,
    image: YuvBiPlanarImageMut<'static, u8>,
    nv12: frame::Video,
    gpu_frames: Option<GpuFrames>,
    pts: i64,
    keyframe_requested: bool,
}

// SAFETY: the FFmpeg contexts inside have no thread affinity, and `&mut self` means only one thread uses them at a time.
unsafe impl Send for FfmpegEncoder {}

fn forced_name(preference: Option<&str>) -> Option<&'static str> {
    match preference {
        Some("vaapi") => Some("h264_vaapi"),
        Some("nvenc") => Some("h264_nvenc"),
        Some("x264") => Some(X264),
        _ => None,
    }
}

fn copy_plane(
    source: &[u8],
    source_stride: usize,
    target: &mut [u8],
    target_stride: usize,
    row_bytes: usize,
) {
    for (to, from) in target
        .chunks_mut(target_stride)
        .zip(source.chunks(source_stride))
    {
        if let (Some(to), Some(from)) = (to.get_mut(..row_bytes), from.get(..row_bytes)) {
            to.copy_from_slice(from);
        }
    }
}

fn x264_preset(width: u32, height: u32, frame_rate: u32) -> &'static str {
    let pixel_rate = u64::from(width) * u64::from(height) * u64::from(frame_rate);
    if pixel_rate <= VERYFAST_MAX_PIXEL_RATE {
        "veryfast"
    } else if pixel_rate <= SUPERFAST_MAX_PIXEL_RATE {
        "superfast"
    } else {
        "ultrafast"
    }
}

fn x264_profile(profile: H264Profile) -> H264Profile {
    match profile {
        H264Profile::ConstrainedBaseline => H264Profile::Baseline,
        other => other,
    }
}

impl FfmpegEncoder {
    pub(crate) fn open_any(
        width: u32,
        height: u32,
        config: EncoderConfig,
        preference: Option<&str>,
    ) -> Result<Option<Self>, Error> {
        let forced = forced_name(preference);
        if let Err(error) = ffmpeg::init() {
            return if forced.is_some() {
                Err(ffmpeg_error(error))
            } else {
                Ok(None)
            };
        }
        ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Fatal);

        let names = forced.map_or_else(|| CANDIDATES.to_vec(), |name| vec![name]);
        for name in names {
            match Self::open(name, width, height, config) {
                Ok(encoder) => return Ok(Some(encoder)),
                Err(error) if forced.is_some() => return Err(named(name, error)),
                Err(_) => {}
            }
        }
        Ok(None)
    }

    fn open(
        name: &'static str,
        width: u32,
        height: u32,
        config: EncoderConfig,
    ) -> Result<Self, Error> {
        let codec = encoder::find_by_name(name).ok_or(Error::Unsupported("FFmpeg encoder"))?;
        let mut video = codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()
            .map_err(ffmpeg_error)?;

        let rate = i32::try_from(config.frame_rate)
            .map_err(|_| Error::Encode("frame rate is too high".into()))?;
        video.set_width(width);
        video.set_height(height);
        video.set_time_base(Rational(1, rate));
        video.set_frame_rate(Some(Rational(rate, 1)));
        video.set_bit_rate(config.bitrate_bps as usize);
        video.set_gop(config.frame_rate);
        video.set_max_b_frames(0);
        video.set_colorspace(color::Space::BT709);
        video.set_color_range(color::Range::MPEG);
        video.set_color_primaries(color::Primaries::BT709);
        video.set_color_transfer_characteristic(color::TransferCharacteristic::BT709);

        let gpu_frames = if name == "h264_vaapi" {
            let gpu_frames = GpuFrames::new(width, height)?;
            video.set_format(Pixel::VAAPI);
            // SAFETY: `video` owns a valid codec context. av_buffer_ref returns a new reference
            // that the context frees when it is closed.
            unsafe { (*video.as_mut_ptr()).hw_frames_ctx = ffi::av_buffer_ref(gpu_frames.frames) };
            Some(gpu_frames)
        } else {
            video.set_format(Pixel::NV12);
            None
        };

        let profile = if name == X264 {
            x264_profile(config.profile)
        } else {
            config.profile
        };
        let mut options = Dictionary::new();
        if let Some(profile) = ffmpeg_profile(profile) {
            options.set("profile", profile);
        }
        if name == X264 {
            let bitrate = u64::from(config.bitrate_bps);
            options.set("preset", x264_preset(width, height, config.frame_rate));
            options.set("tune", "zerolatency");
            options.set("forced-idr", "1");
            options.set("maxrate", &bitrate.to_string());
            options.set("bufsize", &(bitrate * VBV_MILLISECONDS / 1000).to_string());
        }
        if name == "h264_vaapi" {
            options.set("async_depth", "1");
        }
        if name == "h264_nvenc" {
            options.set("preset", "p1");
            options.set("tune", "ull");
            options.set("delay", "0");
            options.set("forced-idr", "1");
        }

        let encoder = video.open_as_with(codec, options).map_err(ffmpeg_error)?;

        Ok(Self {
            name,
            encoder,
            image: convert::nv12_image(width, height),
            nv12: frame::Video::new(Pixel::NV12, width, height),
            gpu_frames,
            pts: 0,
            keyframe_requested: false,
        })
    }

    pub(crate) fn name(&self) -> &'static str {
        self.name
    }

    pub(crate) fn request_keyframe(&mut self) {
        self.keyframe_requested = true;
    }

    pub(crate) fn encode(&mut self, frame: &Frame) -> Result<Vec<u8>, Error> {
        convert::to_nv12(frame, &mut self.image)?;
        let row_bytes = frame.width as usize;
        let (y_stride, uv_stride) = (self.nv12.stride(0), self.nv12.stride(1));
        copy_plane(
            self.image.y_plane.borrow(),
            self.image.y_stride as usize,
            self.nv12.data_mut(0),
            y_stride,
            row_bytes,
        );
        copy_plane(
            self.image.uv_plane.borrow(),
            self.image.uv_stride as usize,
            self.nv12.data_mut(1),
            uv_stride,
            row_bytes,
        );

        let mut input = match &self.gpu_frames {
            Some(gpu_frames) => gpu_frames.upload(&self.nv12)?,
            None => self.nv12.clone(),
        };
        input.set_pts(Some(self.pts));
        self.pts += 1;
        if self.keyframe_requested {
            input.set_kind(picture::Type::I);
            self.keyframe_requested = false;
        }

        self.encoder.send_frame(&input).map_err(ffmpeg_error)?;

        let mut h264 = Vec::new();
        let mut packet = Packet::empty();
        while self.encoder.receive_packet(&mut packet).is_ok() {
            h264.extend_from_slice(packet.data().unwrap_or_default());
        }
        Ok(h264)
    }
}

struct GpuFrames {
    device: *mut ffi::AVBufferRef,
    frames: *mut ffi::AVBufferRef,
}

impl GpuFrames {
    fn new(width: u32, height: u32) -> Result<Self, Error> {
        let mut gpu = Self {
            device: ptr::null_mut(),
            frames: ptr::null_mut(),
        };

        // SAFETY: av_hwdevice_ctx_create writes a new device reference into `gpu.device`
        // or leaves it null on failure. Drop frees whatever was created.
        let ret = unsafe {
            ffi::av_hwdevice_ctx_create(
                &mut gpu.device,
                ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
                ptr::null(),
                ptr::null_mut(),
                0,
            )
        };
        if ret < 0 {
            return Err(Error::Unsupported("VA-API device"));
        }

        // SAFETY: `gpu.device` is a valid device reference. The frames context data is an
        // AVHWFramesContext that we fill in before av_hwframe_ctx_init.
        unsafe {
            gpu.frames = ffi::av_hwframe_ctx_alloc(gpu.device);
            if gpu.frames.is_null() {
                return Err(Error::Unsupported("VA-API frames"));
            }
            let ctx = (*gpu.frames).data as *mut ffi::AVHWFramesContext;
            (*ctx).format = ffi::AVPixelFormat::AV_PIX_FMT_VAAPI;
            (*ctx).sw_format = ffi::AVPixelFormat::AV_PIX_FMT_NV12;
            (*ctx).width = width as i32;
            (*ctx).height = height as i32;
            (*ctx).initial_pool_size = 4;
            if ffi::av_hwframe_ctx_init(gpu.frames) < 0 {
                return Err(Error::Unsupported("VA-API frames"));
            }
        }

        Ok(gpu)
    }

    fn upload(&self, nv12: &frame::Video) -> Result<frame::Video, Error> {
        let mut gpu_frame = frame::Video::empty();
        // SAFETY: `self.frames` is an initialized frames context and both frames are valid.
        // av_hwframe_get_buffer allocates GPU memory owned by `gpu_frame`.
        let ok = unsafe {
            ffi::av_hwframe_get_buffer(self.frames, gpu_frame.as_mut_ptr(), 0) >= 0
                && ffi::av_hwframe_transfer_data(gpu_frame.as_mut_ptr(), nv12.as_ptr(), 0) >= 0
        };
        if ok {
            Ok(gpu_frame)
        } else {
            Err(Error::Encode("uploading frame to the GPU failed".into()))
        }
    }
}

impl Drop for GpuFrames {
    fn drop(&mut self) {
        // SAFETY: both fields are null or references we own. av_buffer_unref accepts null.
        unsafe {
            ffi::av_buffer_unref(&mut self.frames);
            ffi::av_buffer_unref(&mut self.device);
        }
    }
}

fn ffmpeg_profile(profile: H264Profile) -> Option<&'static str> {
    match profile {
        H264Profile::Unspecified => None,
        H264Profile::ConstrainedBaseline => Some("constrained_baseline"),
        H264Profile::Baseline => Some("baseline"),
        H264Profile::Main => Some("main"),
        H264Profile::High => Some("high"),
    }
}

fn named(name: &str, error: Error) -> Error {
    match error {
        Error::Encode(reason) => Error::Encode(format!("{name}: {reason}")),
        other => other,
    }
}

fn ffmpeg_error(error: ffmpeg::Error) -> Error {
    Error::Encode(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    fn x264_available() -> bool {
        ffmpeg::init().is_ok() && encoder::find_by_name(X264).is_some()
    }

    fn nal_types(h264: &[u8]) -> Vec<u8> {
        h264.windows(4)
            .filter(|window| window[..3] == [0, 0, 1])
            .map(|window| window[3] & 0x1F)
            .collect()
    }

    fn sps_profile(h264: &[u8]) -> Option<u8> {
        h264.windows(5)
            .find(|window| window[..3] == [0, 0, 1] && window[3] & 0x1F == 7)
            .map(|window| window[4])
    }

    fn frame(patch: u8) -> Frame {
        let mut data = vec![100; 64 * 64 * 4];
        data[..64 * 4 * 4].fill(patch);
        Frame::new(64, 64, data, Instant::now())
    }

    #[test]
    fn rows_are_copied_without_the_padding_of_either_side() {
        let source = [1, 2, 3, 0, 4, 5, 6, 0];
        let mut target = [9; 10];
        copy_plane(&source, 4, &mut target, 5, 3);
        assert_eq!(target, [1, 2, 3, 9, 9, 4, 5, 6, 9, 9]);
    }

    #[test]
    fn a_named_encoder_is_the_only_candidate() {
        assert_eq!(forced_name(Some("vaapi")), Some("h264_vaapi"));
        assert_eq!(forced_name(Some("nvenc")), Some("h264_nvenc"));
        assert_eq!(forced_name(Some("x264")), Some("libx264"));
    }

    #[test]
    fn a_failed_encoder_is_named_in_its_error() {
        let error = named(
            "h264_nvenc",
            Error::Encode("Operation not permitted".into()),
        );
        assert_eq!(
            error.to_string(),
            "H.264 encoding failed: h264_nvenc: Operation not permitted"
        );
        assert!(matches!(
            named("h264_nvenc", Error::InvalidFrame),
            Error::InvalidFrame
        ));
    }

    #[test]
    fn x264_is_tried_after_the_gpu_encoders() {
        assert_eq!(CANDIDATES, ["h264_vaapi", "h264_nvenc", "libx264"]);
    }

    #[test]
    fn the_x264_preset_follows_the_pixel_rate() {
        assert_eq!(x264_preset(1280, 720, 60), "veryfast");
        assert_eq!(x264_preset(1920, 1080, 60), "veryfast");
        assert_eq!(x264_preset(2560, 1440, 60), "superfast");
        assert_eq!(x264_preset(3840, 2160, 30), "superfast");
        assert_eq!(x264_preset(3840, 2160, 60), "ultrafast");
    }

    #[test]
    fn x264_baseline_covers_constrained_baseline() {
        assert_eq!(
            x264_profile(H264Profile::ConstrainedBaseline),
            H264Profile::Baseline
        );
        assert_eq!(x264_profile(H264Profile::High), H264Profile::High);
        assert_eq!(
            x264_profile(H264Profile::Unspecified),
            H264Profile::Unspecified
        );
    }

    #[test]
    fn x264_makes_a_full_picture_on_request() {
        if !x264_available() {
            return;
        }
        let mut encoder = FfmpegEncoder::open(X264, 64, 64, EncoderConfig::default()).unwrap();
        assert_eq!(encoder.name(), "libx264");

        let first = encoder.encode(&frame(100)).unwrap();
        assert!(nal_types(&first).contains(&5));
        let second = encoder.encode(&frame(110)).unwrap();
        assert!(!nal_types(&second).contains(&5));
        encoder.request_keyframe();
        let third = encoder.encode(&frame(120)).unwrap();
        assert!(nal_types(&third).contains(&5));
    }

    #[test]
    fn x264_writes_the_requested_profile() {
        if !x264_available() {
            return;
        }
        for (profile, expected) in [
            (H264Profile::ConstrainedBaseline, 66),
            (H264Profile::Baseline, 66),
            (H264Profile::Main, 77),
            (H264Profile::High, 100),
        ] {
            let config = EncoderConfig {
                profile,
                ..EncoderConfig::default()
            };
            let mut encoder = FfmpegEncoder::open(X264, 64, 64, config).unwrap();
            let bytes = encoder.encode(&frame(100)).unwrap();
            assert_eq!(sps_profile(&bytes), Some(expected), "{profile:?}");
        }
    }

    #[test]
    fn no_or_an_unknown_name_leaves_the_choice_to_the_library() {
        assert_eq!(forced_name(None), None);
        assert_eq!(forced_name(Some("openh264")), None);
        assert_eq!(forced_name(Some("nonsense")), None);
    }
}
