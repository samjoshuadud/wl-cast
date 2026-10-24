mod ext_image_copy;
mod wlr_screencopy;

use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    protocol::{
        wl_buffer::WlBuffer,
        wl_output::WlOutput,
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

#[derive(Default)]
struct State {
    outputs: Vec<WlOutput>,
    shm: Option<WlShm>,
    frame_ready: bool,
    frame_failed: bool,

    ext_source_manager: Option<ExtOutputImageCaptureSourceManagerV1>,
    ext_capture_manager: Option<ExtImageCopyCaptureManagerV1>,
    ext_width: u32,
    ext_height: u32,
    ext_formats: Vec<wl_shm::Format>,
    ext_constraints_done: bool,

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
                "wl_output" => state.outputs.push(registry.bind(name, 1, qh, ())),
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

delegate_noop!(State: ignore WlOutput);
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
}
