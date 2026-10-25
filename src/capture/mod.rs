mod ext_image_copy;
#[cfg(feature = "portal")]
mod portal;
mod wlr_screencopy;

use std::{
    fmt,
    fs::File,
    io::ErrorKind,
    os::fd::AsFd,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use rustix::fs::{MemfdFlags, memfd_create};
use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    io::Errno,
};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, WEnum,
    backend::WaylandError,
    delegate_noop,
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

const STOP_WAIT: Timespec = Timespec {
    tv_sec: 0,
    tv_nsec: 200_000_000,
};

/// Stops a blocking [`Capture::next_frame`] or a remote session from another thread.
#[derive(Clone, Debug)]
pub struct StopHandle {
    stop: Arc<AtomicBool>,
}

impl StopHandle {
    /// Creates a handle that is not stopped.
    pub fn new() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Unblocks whoever is waiting on this handle.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    #[cfg(any(test, feature = "webrtc"))]
    pub(crate) fn is_stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    pub(crate) fn flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stop)
    }
}

impl Default for StopHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// Captures frames from one monitor.
pub struct Capture {
    backend: Backend,
}

enum Backend {
    Ext(ext_image_copy::ExtCapture),
    Wlr(wlr_screencopy::WlrCapture),
    #[cfg(feature = "portal")]
    Portal(portal::PortalCapture),
}

#[derive(Debug, PartialEq, Eq)]
enum Kind {
    Ext,
    Wlr,
    #[cfg(feature = "portal")]
    Portal,
}

impl Capture {
    /// Starts capturing the monitor at `output_index`, where 0 is the first monitor.
    ///
    /// Uses ext-image-copy-capture when the compositor offers it, otherwise the older wlr-screencopy.
    /// With the `portal` feature, compositors that offer neither, such as GNOME and KDE, go through
    /// xdg-desktop-portal. The desktop then asks which monitor to share, `output_index` is not used,
    /// and this call waits for that choice.
    /// Set the `WL_CAST_CAPTURE` environment variable to `ext`, `wlr` or `portal` to force one.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Connect`] if there is no compositor, [`Error::Unsupported`] if no capture
    /// method is available, [`Error::OutputNotFound`] if there is no monitor at `output_index`,
    /// or `Error::Portal` if the portal or PipeWire fails or the choice is cancelled.
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
            #[cfg(feature = "portal")]
            Kind::Portal => Backend::Portal(portal::PortalCapture::new()?),
        };
        Ok(Self { backend })
    }

    /// Returns the width and height in pixels of the latest frame.
    ///
    /// Before the first frame, this is the monitor size at the moment capturing started.
    /// It changes after the monitor is resized or rotated.
    pub fn size(&self) -> (u32, u32) {
        match &self.backend {
            Backend::Ext(capture) => capture.size(),
            Backend::Wlr(capture) => capture.size(),
            #[cfg(feature = "portal")]
            Backend::Portal(capture) => capture.size(),
        }
    }

    /// Unblocks [`Capture::next_frame`] when `handle` is stopped, even if the screen is idle.
    ///
    /// Without this, [`Capture::next_frame`] waits until the screen changes.
    pub fn set_stop_handle(&mut self, handle: &StopHandle) {
        let flag = handle.flag();
        match &mut self.backend {
            Backend::Ext(capture) => capture.set_stop(flag),
            Backend::Wlr(capture) => capture.set_stop(flag),
            #[cfg(feature = "portal")]
            Backend::Portal(capture) => capture.set_stop(flag),
        }
    }

    /// Waits for the next frame, which blocks while the screen is idle.
    ///
    /// # Errors
    ///
    /// Returns [`Error::CaptureFailed`] if the compositor cannot capture, or [`Error::Stopped`]
    /// if a handle from [`Capture::set_stop_handle`] was stopped.
    pub fn next_frame(&mut self) -> Result<Frame, Error> {
        match &mut self.backend {
            Backend::Ext(capture) => capture.next_frame(),
            Backend::Wlr(capture) => capture.next_frame(),
            #[cfg(feature = "portal")]
            Backend::Portal(capture) => capture.next_frame(),
        }
    }
}

impl fmt::Debug for Capture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let backend = match self.backend {
            Backend::Ext(_) => "ext-image-copy-capture",
            Backend::Wlr(_) => "wlr-screencopy",
            #[cfg(feature = "portal")]
            Backend::Portal(_) => "xdg-desktop-portal",
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
        Some("portal") => portal_kind().ok_or(Error::Unsupported(
            "xdg-desktop-portal capture without the portal feature",
        )),
        _ if ext => Ok(Kind::Ext),
        _ if wlr => Ok(Kind::Wlr),
        _ => portal_kind().ok_or(Error::Unsupported(
            "ext-image-copy-capture or wlr-screencopy",
        )),
    }
}

fn portal_kind() -> Option<Kind> {
    #[cfg(feature = "portal")]
    {
        Some(Kind::Portal)
    }
    #[cfg(not(feature = "portal"))]
    {
        None
    }
}

fn wait_for(
    queue: &mut EventQueue<State>,
    state: &mut State,
    stop: Option<&AtomicBool>,
    ready: impl Fn(&State) -> bool,
) -> Result<(), Error> {
    if stop.is_none() {
        while !ready(state) {
            queue.blocking_dispatch(state)?;
        }
        return Ok(());
    }

    loop {
        if stop.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err(Error::Stopped);
        }
        if ready(state) {
            return Ok(());
        }
        if queue.dispatch_pending(state)? > 0 {
            continue;
        }
        queue
            .flush()
            .map_err(|error| Error::Dispatch(error.to_string()))?;
        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        let fd = guard.connection_fd();
        let mut fds = [PollFd::new(&fd, PollFlags::IN | PollFlags::ERR)];
        match poll(&mut fds, Some(&STOP_WAIT)) {
            Ok(0) => continue,
            Ok(_) => match guard.read() {
                Ok(_) => {}
                Err(WaylandError::Io(error)) if error.kind() == ErrorKind::WouldBlock => {}
                Err(error) => return Err(Error::Dispatch(error.to_string())),
            },
            Err(Errno::INTR) => continue,
            Err(error) => return Err(Error::Io(error.into())),
        }
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

fn pack_rows(raw: &[u8], width: u32, stride: u32, y_invert: bool) -> Vec<u8> {
    let row = width as usize * 4;
    let mut rows: Vec<&[u8]> = raw
        .chunks_exact(stride as usize)
        .filter_map(|chunk| chunk.get(..row))
        .collect();
    if y_invert {
        rows.reverse();
    }
    rows.concat()
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
    ext_constraint_epoch: u32,
    ext_constraints_mismatch: bool,
    ext_stopped: bool,
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

    fn begin_ext_constraints(&mut self) {
        if self.ext_constraints_done {
            self.ext_constraints_done = false;
            self.ext_formats.clear();
        }
    }

    fn ext_buffer_size(&mut self, width: u32, height: u32) {
        self.begin_ext_constraints();
        self.ext_width = width;
        self.ext_height = height;
    }

    fn ext_shm_format(&mut self, format: wl_shm::Format) {
        self.begin_ext_constraints();
        self.ext_formats.push(format);
    }

    fn ext_constraints_finished(&mut self) {
        self.ext_constraints_done = true;
        self.ext_constraint_epoch = self.ext_constraint_epoch.wrapping_add(1);
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
    fn padding_at_the_end_of_each_row_is_removed() {
        let raw = [
            1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0, 9, 10, 11, 12, 13, 14, 15, 16, 0, 0, 0, 0,
        ];
        let packed = pack_rows(&raw, 2, 12, false);
        assert_eq!(
            packed,
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
        );
    }

    #[test]
    fn rows_are_flipped_when_the_frame_is_upside_down() {
        let raw = [1, 1, 1, 1, 2, 2, 2, 2];
        assert_eq!(pack_rows(&raw, 1, 4, true), [2, 2, 2, 2, 1, 1, 1, 1]);
        assert_eq!(pack_rows(&raw, 1, 4, false), raw);
    }

    #[test]
    fn stopping_a_clone_stops_the_original_handle() {
        let handle = StopHandle::new();
        let clone = handle.clone();
        assert!(!handle.is_stopped());
        clone.stop();
        assert!(handle.is_stopped());
    }

    #[test]
    fn prefers_ext_when_both_are_offered() {
        assert!(matches!(choose(None, true, true), Ok(Kind::Ext)));
    }

    #[test]
    fn falls_back_to_wlr_when_ext_is_missing() {
        assert!(matches!(choose(None, false, true), Ok(Kind::Wlr)));
    }

    #[cfg(not(feature = "portal"))]
    #[test]
    fn fails_when_nothing_is_offered() {
        assert!(matches!(
            choose(None, false, false),
            Err(Error::Unsupported(_))
        ));
    }

    #[cfg(not(feature = "portal"))]
    #[test]
    fn forcing_the_portal_needs_the_portal_feature() {
        assert!(matches!(
            choose(Some("portal"), true, true),
            Err(Error::Unsupported(_))
        ));
    }

    #[cfg(feature = "portal")]
    #[test]
    fn falls_back_to_the_portal_when_nothing_is_offered() {
        assert!(matches!(choose(None, false, false), Ok(Kind::Portal)));
    }

    #[cfg(feature = "portal")]
    #[test]
    fn the_portal_can_be_forced_when_other_backends_exist() {
        assert!(matches!(
            choose(Some("portal"), true, true),
            Ok(Kind::Portal)
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
    fn a_new_constraint_batch_replaces_the_previous_one() {
        let mut state = State::default();
        state.ext_buffer_size(10, 20);
        state.ext_shm_format(wl_shm::Format::Xrgb8888);
        state.ext_shm_format(wl_shm::Format::Argb8888);
        state.ext_constraints_finished();
        assert_eq!(state.ext_constraint_epoch, 1);
        assert_eq!(
            state.ext_formats,
            vec![wl_shm::Format::Xrgb8888, wl_shm::Format::Argb8888]
        );

        state.ext_shm_format(wl_shm::Format::Xbgr8888);
        assert!(!state.ext_constraints_done);
        assert_eq!(state.ext_formats, vec![wl_shm::Format::Xbgr8888]);
        state.ext_buffer_size(30, 40);
        assert_eq!(state.ext_formats, vec![wl_shm::Format::Xbgr8888]);
        state.ext_constraints_finished();
        assert_eq!((state.ext_width, state.ext_height), (30, 40));
        assert_eq!(state.ext_constraint_epoch, 2);
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
