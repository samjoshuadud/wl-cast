mod ext_image_copy;
mod wlr_screencopy;

use std::{fmt, fs::File, os::fd::AsFd};

use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum, delegate_noop,
    protocol::{
        wl_buffer::WlBuffer,
        wl_output::{self, Transform, WlOutput},
        wl_registry::{self, WlRegistry},
        wl_shm::{self, WlShm},
        wl_shm_pool::WlShmPool,
    },
};
use wayland_protocols::ext::{
    image_capture_source::v1::client::{
        ext_image_capture_source_v1::ExtImageCaptureSourceV1,
        ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
    },
    image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1,
};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;

use crate::{Error, Frame};

/// Captures frames from one monitor.
pub struct Capture {
    backend: Backend,
}

enum Backend {
    Ext(ext_image_copy::ExtCapture),
    Wlr(wlr_screencopy::WlrCapture),
}

#[derive(Debug, PartialEq, Eq)]
enum Kind {
    Ext,
    Wlr,
}

impl Capture {
    /// Starts capturing the monitor at `output_index`, where 0 is the first monitor.
    ///
    /// Uses ext-image-copy-capture when the compositor offers it, otherwise the older wlr-screencopy.
    /// Set the `WL_CAST_CAPTURE` environment variable to `ext` or `wlr` to force one.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Connect`] if there is no compositor, [`Error::Unsupported`] if it offers
    /// neither protocol, or [`Error::OutputNotFound`] if there is no monitor at `output_index`.
    pub fn new(output_index: usize) -> Result<Self, Error> {
        let conn = Connection::connect_to_env()?;
        let mut queue = conn.new_event_queue::<State>();
        let qh = queue.handle();
        let mut state = State::default();

        conn.display().get_registry(&qh, ());
        queue.roundtrip(&mut state)?;
        queue.roundtrip(&mut state)?;

        let preference = std::env::var("WL_CAST_CAPTURE").ok();
        let backend = match choose(preference.as_deref(), state.has_ext(), state.has_wlr())? {
            Kind::Ext => Backend::Ext(ext_image_copy::ExtCapture::new(queue, state, output_index)?),
            Kind::Wlr => Backend::Wlr(wlr_screencopy::WlrCapture::new(queue, state, output_index)?),
        };
        Ok(Self { backend })
    }

    /// Returns the width and height in pixels of every frame.
    pub fn size(&self) -> (u32, u32) {
        match &self.backend {
            Backend::Ext(capture) => capture.size(),
            Backend::Wlr(capture) => capture.size(),
        }
    }

    /// Waits for the next frame, which blocks while the screen is idle.
    ///
    /// # Errors
    ///
    /// Returns [`Error::CaptureFailed`] if the compositor cannot capture, for example after the monitor changes.
    pub fn next_frame(&mut self) -> Result<Frame, Error> {
        match &mut self.backend {
            Backend::Ext(capture) => capture.next_frame(),
            Backend::Wlr(capture) => capture.next_frame(),
        }
    }
}

impl fmt::Debug for Capture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let backend = match self.backend {
            Backend::Ext(_) => "ext-image-copy-capture",
            Backend::Wlr(_) => "wlr-screencopy",
        };
        f.debug_struct("Capture")
            .field("backend", &backend)
            .field("size", &self.size())
            .finish()
    }
}

fn choose(preference: Option<&str>, ext: bool, wlr: bool) -> Result<Kind, Error> {
    match preference {
        Some("ext") if ext => Ok(Kind::Ext),
        Some("ext") => Err(Error::Unsupported("ext-image-copy-capture")),
        Some("wlr") if wlr => Ok(Kind::Wlr),
        Some("wlr") => Err(Error::Unsupported("wlr-screencopy")),
        _ if ext => Ok(Kind::Ext),
        _ if wlr => Ok(Kind::Wlr),
        _ => Err(Error::Unsupported(
            "ext-image-copy-capture or wlr-screencopy",
        )),
    }
}

const FORMATS: [wl_shm::Format; 4] = [
    wl_shm::Format::Xrgb8888,
    wl_shm::Format::Argb8888,
    wl_shm::Format::Xbgr8888,
    wl_shm::Format::Abgr8888,
];

const FORMATS_ERROR: &str = "Xrgb8888, Argb8888, Xbgr8888 or Abgr8888 shared memory buffers";

fn to_bgrx(mut data: Vec<u8>, format: wl_shm::Format) -> Vec<u8> {
    if matches!(format, wl_shm::Format::Xbgr8888 | wl_shm::Format::Abgr8888) {
        for pixel in data.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
    }
    data
}

fn shm_buffer(
    shm: &WlShm,
    qh: &QueueHandle<State>,
    width: u32,
    height: u32,
    stride: u32,
    format: wl_shm::Format,
) -> Result<(File, WlBuffer), Error> {
    let size = stride as usize * height as usize;
    let fd = memfd_create("wl-cast-frame", MemfdFlags::CLOEXEC).map_err(std::io::Error::from)?;
    let file = File::from(fd);
    file.set_len(size as u64)?;
    let pool = shm.create_pool(file.as_fd(), size as i32, qh, ());
    let buffer = pool.create_buffer(
        0,
        width as i32,
        height as i32,
        stride as i32,
        format,
        qh,
        (),
    );
    Ok((file, buffer))
}

fn oriented_size(width: u32, height: u32, transform: Transform) -> (u32, u32) {
    if quarter_turns(transform) % 2 == 1 {
        (height, width)
    } else {
        (width, height)
    }
}

fn quarter_turns(transform: Transform) -> u32 {
    u32::from(transform) & 3
}

fn is_mirrored(transform: Transform) -> bool {
    u32::from(transform) & 4 != 0
}

fn orient(data: Vec<u8>, width: u32, height: u32, transform: Transform) -> Vec<u8> {
    if transform == Transform::Normal {
        return data;
    }
    let (out_width, out_height) = oriented_size(width, height, transform);
    let (width, height) = (i64::from(width), i64::from(height));
    let (out_width, out_height) = (i64::from(out_width), i64::from(out_height));
    let mut out = vec![0; data.len()];
    for (index, pixel) in (0..).zip(data.as_chunks::<4>().0) {
        let mut x = 2 * (index % width) + 1 - width;
        let mut y = 2 * (index / width) + 1 - height;
        for _ in 0..quarter_turns(transform) {
            (x, y) = (-y, x);
        }
        if is_mirrored(transform) {
            x = -x;
        }
        let out_x = (x + out_width - 1) / 2;
        let out_y = (y + out_height - 1) / 2;
        let start = ((out_y * out_width + out_x) * 4) as usize;
        if let Some(target) = out.get_mut(start..start + 4) {
            target.copy_from_slice(pixel);
        }
    }
    out
}

#[derive(Default)]
struct State {
    outputs: Vec<WlOutput>,
    output_transforms: Vec<Transform>,
    shm: Option<WlShm>,
    frame_ready: bool,
    frame_failed: bool,

    ext_source_manager: Option<ExtOutputImageCaptureSourceManagerV1>,
    ext_capture_manager: Option<ExtImageCopyCaptureManagerV1>,
    ext_width: u32,
    ext_height: u32,
    ext_formats: Vec<wl_shm::Format>,
    ext_constraints_done: bool,
    ext_transform: Option<Transform>,

    wlr_manager: Option<ZwlrScreencopyManagerV1>,
    wlr_version: u32,
    wlr_buffer: Option<wlr_screencopy::BufferInfo>,
    wlr_buffer_done: bool,
    wlr_y_invert: bool,
}

impl State {
    fn has_ext(&self) -> bool {
        self.ext_source_manager.is_some() && self.ext_capture_manager.is_some()
    }

    fn has_wlr(&self) -> bool {
        self.wlr_manager.is_some()
    }

    fn output_transform(&self, index: usize) -> Transform {
        self.output_transforms
            .get(index)
            .copied()
            .unwrap_or(Transform::Normal)
    }
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_output" => {
                    let index = state.outputs.len();
                    state.outputs.push(registry.bind(name, 1, qh, index));
                    state.output_transforms.push(Transform::Normal);
                }
                "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
                "ext_output_image_capture_source_manager_v1" => {
                    state.ext_source_manager = Some(registry.bind(name, 1, qh, ()))
                }
                "ext_image_copy_capture_manager_v1" => {
                    state.ext_capture_manager = Some(registry.bind(name, 1, qh, ()))
                }
                "zwlr_screencopy_manager_v1" => {
                    state.wlr_version = version.min(3);
                    state.wlr_manager = Some(registry.bind(name, state.wlr_version, qh, ()))
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<WlOutput, usize> for State {
    fn event(
        state: &mut Self,
        _output: &WlOutput,
        event: wl_output::Event,
        index: &usize,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Geometry {
            transform: WEnum::Value(transform),
            ..
        } = event
            && let Some(slot) = state.output_transforms.get_mut(*index)
        {
            *slot = transform;
        }
    }
}

delegate_noop!(State: ignore WlShm);
delegate_noop!(State: WlShmPool);
delegate_noop!(State: ignore WlBuffer);
delegate_noop!(State: ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(State: ExtImageCaptureSourceV1);
delegate_noop!(State: ExtImageCopyCaptureManagerV1);
delegate_noop!(State: ZwlrScreencopyManagerV1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_ext_when_both_are_offered() {
        assert!(matches!(choose(None, true, true), Ok(Kind::Ext)));
    }

    #[test]
    fn falls_back_to_wlr_when_ext_is_missing() {
        assert!(matches!(choose(None, false, true), Ok(Kind::Wlr)));
    }

    #[test]
    fn fails_when_nothing_is_offered() {
        assert!(matches!(
            choose(None, false, false),
            Err(Error::Unsupported(_))
        ));
    }

    #[test]
    fn a_forced_backend_wins_when_available() {
        assert!(matches!(choose(Some("wlr"), true, true), Ok(Kind::Wlr)));
        assert!(matches!(choose(Some("ext"), true, true), Ok(Kind::Ext)));
    }

    #[test]
    fn a_forced_backend_that_is_missing_is_an_error() {
        assert!(matches!(
            choose(Some("ext"), false, true),
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            choose(Some("wlr"), true, false),
            Err(Error::Unsupported(_))
        ));
    }

    #[test]
    fn an_unknown_preference_is_ignored() {
        assert!(matches!(choose(Some("other"), false, true), Ok(Kind::Wlr)));
    }

    #[test]
    fn an_unknown_output_has_no_transform() {
        let state = State {
            output_transforms: vec![Transform::_90],
            ..State::default()
        };
        assert_eq!(state.output_transform(0), Transform::_90);
        assert_eq!(state.output_transform(1), Transform::Normal);
    }

    #[test]
    fn red_and_blue_are_swapped_only_for_bgr_formats() {
        let pixel = vec![1, 2, 3, 4];
        for format in [wl_shm::Format::Xbgr8888, wl_shm::Format::Abgr8888] {
            assert_eq!(to_bgrx(pixel.clone(), format), [3, 2, 1, 4]);
        }
        for format in [wl_shm::Format::Xrgb8888, wl_shm::Format::Argb8888] {
            assert_eq!(to_bgrx(pixel.clone(), format), pixel);
        }
    }

    fn pixels(values: &[u8]) -> Vec<u8> {
        values.iter().flat_map(|&value| [value; 4]).collect()
    }

    fn oriented(transform: Transform) -> Vec<u8> {
        orient(pixels(&[1, 2, 3, 4, 5, 6]), 3, 2, transform)
    }

    #[test]
    fn a_normal_frame_is_unchanged() {
        assert_eq!(oriented(Transform::Normal), pixels(&[1, 2, 3, 4, 5, 6]));
    }

    #[test]
    fn half_turns_and_mirrors_keep_the_size() {
        assert_eq!(oriented(Transform::_180), pixels(&[6, 5, 4, 3, 2, 1]));
        assert_eq!(oriented(Transform::Flipped), pixels(&[3, 2, 1, 6, 5, 4]));
        assert_eq!(oriented(Transform::Flipped180), pixels(&[4, 5, 6, 1, 2, 3]));
        assert_eq!(oriented_size(3, 2, Transform::Flipped180), (3, 2));
    }

    #[test]
    fn quarter_turns_swap_width_and_height() {
        assert_eq!(oriented(Transform::_90), pixels(&[4, 1, 5, 2, 6, 3]));
        assert_eq!(oriented(Transform::_270), pixels(&[3, 6, 2, 5, 1, 4]));
        assert_eq!(oriented(Transform::Flipped90), pixels(&[1, 4, 2, 5, 3, 6]));
        assert_eq!(oriented(Transform::Flipped270), pixels(&[6, 3, 5, 2, 4, 1]));
        assert_eq!(oriented_size(3, 2, Transform::_90), (2, 3));
    }
}
