use std::ptr;

use ffmpeg_next::{
    self as ffmpeg, Dictionary, Packet, Rational, codec, encoder, ffi, format::Pixel, frame,
    picture, software::scaling,
};

use super::{EncoderConfig, H264Profile};
use crate::{Error, Frame};

const CANDIDATES: [&str; 2] = ["h264_vaapi", "h264_nvenc"];

pub(crate) struct FfmpegEncoder {
    name: &'static str,
    encoder: encoder::Video,
    scaler: scaling::Context,
    bgrx: frame::Video,
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
        _ => None,
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
                Err(error) if forced.is_some() => return Err(error),
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

        let mut options = Dictionary::new();
        if let Some(profile) = ffmpeg_profile(config.profile) {
            options.set("profile", profile);
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
        let scaler = scaling::Context::get(
            Pixel::BGRZ,
            width,
            height,
            Pixel::NV12,
            width,
            height,
            scaling::Flags::FAST_BILINEAR,
        )
        .map_err(ffmpeg_error)?;

        Ok(Self {
            name,
            encoder,
            scaler,
            bgrx: frame::Video::new(Pixel::BGRZ, width, height),
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
        let row = frame.width as usize * 4;
        let stride = self.bgrx.stride(0);
        let plane = self.bgrx.data_mut(0);
        for (y, src) in frame.data.chunks_exact(row).enumerate() {
            plane[y * stride..y * stride + row].copy_from_slice(src);
        }

        self.scaler
            .run(&self.bgrx, &mut self.nv12)
            .map_err(ffmpeg_error)?;

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

fn ffmpeg_error(error: ffmpeg::Error) -> Error {
    Error::Encode(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_encoder_is_the_only_candidate() {
        assert_eq!(forced_name(Some("vaapi")), Some("h264_vaapi"));
        assert_eq!(forced_name(Some("nvenc")), Some("h264_nvenc"));
    }

    #[test]
    fn no_or_an_unknown_name_leaves_the_choice_to_the_library() {
        assert_eq!(forced_name(None), None);
        assert_eq!(forced_name(Some("openh264")), None);
        assert_eq!(forced_name(Some("nonsense")), None);
    }
}
