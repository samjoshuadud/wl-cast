use std::{
    fmt::Display,
    io::Cursor,
    os::fd::OwnedFd,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use ashpd::{
    desktop::{
        PersistMode, Session,
        screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType},
    },
    enumflags2::BitFlags,
};
use futures_lite::future::block_on;
use pipewire::{
    self as pw,
    properties::properties,
    spa::{
        self,
        param::{
            ParamType,
            format::{FormatProperties, MediaSubtype, MediaType},
            format_utils::parse_format,
            video::{VideoFormat, VideoInfoRaw},
        },
        pod::{Pod, Value, serialize::PodSerializer},
        utils::{Direction, Fraction, Rectangle, SpaTypes},
    },
    stream::{Stream, StreamBox, StreamFlags, StreamState},
};
use wayland_client::protocol::wl_shm;

use super::{pack_rows, to_bgrx};
use crate::{Error, Frame};

const FORMAT_TIMEOUT: Duration = Duration::from_secs(10);
const STOP_CHECK: Duration = Duration::from_millis(200);
const MAX_SIZE: u32 = 16_384;

pub(super) struct PortalCapture {
    session: Session<Screencast>,
    shared: Arc<Shared>,
    quit: pw::channel::Sender<()>,
    thread: Option<JoinHandle<()>>,
    size: (u32, u32),
    stop: Option<Arc<AtomicBool>>,
}

#[derive(Default)]
struct Shared {
    latest: Mutex<Latest>,
    changed: Condvar,
}

#[derive(Default)]
struct Latest {
    frame: Option<Frame>,
    size: Option<(u32, u32)>,
    failure: Option<String>,
}

impl PortalCapture {
    pub(super) fn new() -> Result<Self, Error> {
        let (session, node, size_hint, fd) = block_on(start_portal())?;
        let shared = Arc::new(Shared::default());
        let (quit, quit_receiver) = pw::channel::channel();
        let thread = thread::spawn({
            let shared = Arc::clone(&shared);
            move || {
                if let Err(error) = stream_frames(fd, node, size_hint, &shared, quit_receiver) {
                    shared.fail(error.to_string());
                }
            }
        });
        let mut capture = Self {
            session,
            shared,
            quit,
            thread: Some(thread),
            size: (0, 0),
            stop: None,
        };
        let deadline = Instant::now() + FORMAT_TIMEOUT;
        capture.size = capture
            .shared
            .wait(None, Some(deadline), |latest| latest.size)?;
        Ok(capture)
    }

    pub(super) fn size(&self) -> (u32, u32) {
        self.size
    }

    pub(super) fn set_stop(&mut self, stop: Arc<AtomicBool>) {
        self.stop = Some(stop);
    }

    pub(super) fn next_frame(&mut self) -> Result<Frame, Error> {
        let frame = self
            .shared
            .wait(self.stop.as_deref(), None, |latest| latest.frame.take())?;
        self.size = (frame.width, frame.height);
        Ok(frame)
    }
}

impl Drop for PortalCapture {
    fn drop(&mut self) {
        let _ = self.quit.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = block_on(self.session.close());
    }
}

impl Shared {
    fn update(&self, change: impl FnOnce(&mut Latest)) {
        if let Ok(mut latest) = self.latest.lock() {
            change(&mut latest);
        }
        self.changed.notify_all();
    }

    fn fail(&self, reason: String) {
        self.update(|latest| latest.failure = Some(reason));
    }

    fn wait<T>(
        &self,
        stop: Option<&AtomicBool>,
        deadline: Option<Instant>,
        mut take: impl FnMut(&mut Latest) -> Option<T>,
    ) -> Result<T, Error> {
        let mut latest = self.latest.lock().map_err(|_| Error::CaptureFailed)?;
        loop {
            if let Some(reason) = &latest.failure {
                return Err(Error::Portal(reason.clone()));
            }
            if let Some(value) = take(&mut latest) {
                return Ok(value);
            }
            if stop.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                return Err(Error::Stopped);
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(Error::Portal("PipeWire sent no video format".into()));
            }
            latest = self
                .changed
                .wait_timeout(latest, STOP_CHECK)
                .map_err(|_| Error::CaptureFailed)?
                .0;
        }
    }
}

async fn start_portal() -> Result<(Session<Screencast>, u32, Option<(u32, u32)>, OwnedFd), Error> {
    let proxy = Screencast::new().await.map_err(portal_error)?;
    let cursor = proxy
        .available_cursor_modes()
        .await
        .ok()
        .filter(|modes| modes.contains(CursorMode::Embedded))
        .map(|_| CursorMode::Embedded);
    let session = proxy
        .create_session(Default::default())
        .await
        .map_err(portal_error)?;
    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_cursor_mode(cursor)
                .set_sources(BitFlags::from_flag(SourceType::Monitor))
                .set_multiple(false)
                .set_persist_mode(PersistMode::DoNot),
        )
        .await
        .map_err(portal_error)?
        .response()
        .map_err(portal_error)?;
    let streams = proxy
        .start(&session, None, Default::default())
        .await
        .map_err(portal_error)?
        .response()
        .map_err(portal_error)?;
    let stream = streams
        .streams()
        .first()
        .ok_or_else(|| Error::Portal("the portal shared no monitor".into()))?;
    let size_hint = stream.size().and_then(|(width, height)| {
        Some((u32::try_from(width).ok()?, u32::try_from(height).ok()?))
    });
    let fd = proxy
        .open_pipe_wire_remote(&session, Default::default())
        .await
        .map_err(portal_error)?;
    Ok((session, stream.pipe_wire_node_id(), size_hint, fd))
}

fn stream_frames(
    fd: OwnedFd,
    node: u32,
    size_hint: Option<(u32, u32)>,
    shared: &Arc<Shared>,
    quit: pw::channel::Receiver<()>,
) -> Result<(), Error> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(portal_error)?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(portal_error)?;
    let core = context.connect_fd_rc(fd, None).map_err(portal_error)?;
    let stream = StreamBox::new(
        &core,
        "wl-cast",
        properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )
    .map_err(portal_error)?;

    let _quit = quit.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        move |()| mainloop.quit()
    });
    let _listener = stream
        .add_local_listener_with_user_data(VideoInfoRaw::new())
        .state_changed({
            let shared = Arc::clone(shared);
            move |_, _, _, state| match state {
                StreamState::Error(reason) => shared.fail(reason),
                StreamState::Unconnected => shared.fail("the screen cast ended".into()),
                _ => {}
            }
        })
        .param_changed({
            let shared = Arc::clone(shared);
            move |_, format, id, param| {
                if let Some(size) = negotiated_size(format, id, param) {
                    shared.update(|latest| latest.size = Some(size));
                }
            }
        })
        .process({
            let shared = Arc::clone(shared);
            move |stream, format| {
                if let Some(frame) = take_frame(stream, format) {
                    shared.update(|latest| latest.frame = Some(frame));
                }
            }
        })
        .register()
        .map_err(portal_error)?;

    let params = format_params(size_hint)?;
    let pod = Pod::from_bytes(&params).ok_or_else(|| Error::Portal("bad format request".into()))?;
    stream
        .connect(
            Direction::Input,
            Some(node),
            StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
            &mut [pod],
        )
        .map_err(portal_error)?;
    mainloop.run();
    Ok(())
}

fn negotiated_size(format: &mut VideoInfoRaw, id: u32, param: Option<&Pod>) -> Option<(u32, u32)> {
    let param = param?;
    if id != ParamType::Format.as_raw() {
        return None;
    }
    let (media_type, media_subtype) = parse_format(param).ok()?;
    if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
        return None;
    }
    format.parse(param).ok()?;
    let size = format.size();
    Some((size.width, size.height))
}

fn take_frame(stream: &Stream, format: &VideoInfoRaw) -> Option<Frame> {
    let shm_format = shm_format(format.format())?;
    let Rectangle { width, height } = format.size();
    let mut buffer = stream.dequeue_buffer()?;
    let data = buffer.datas_mut().first_mut()?;
    let chunk = data.chunk();
    let (offset, length, stride) = (chunk.offset(), chunk.size(), chunk.stride());
    if length == 0 || stride == 0 {
        return None;
    }
    let bytes = data
        .data()?
        .get(offset as usize..offset as usize + length as usize)?;
    let rows = pack_rows(bytes, width, stride.unsigned_abs(), stride < 0);
    if rows.len() != width as usize * height as usize * 4 {
        return None;
    }
    Some(Frame {
        width,
        height,
        data: to_bgrx(rows, shm_format),
        captured_at: Instant::now(),
    })
}

fn shm_format(format: VideoFormat) -> Option<wl_shm::Format> {
    match format {
        VideoFormat::BGRx => Some(wl_shm::Format::Xrgb8888),
        VideoFormat::BGRA => Some(wl_shm::Format::Argb8888),
        VideoFormat::RGBx => Some(wl_shm::Format::Xbgr8888),
        VideoFormat::RGBA => Some(wl_shm::Format::Abgr8888),
        _ => None,
    }
}

fn format_params(size_hint: Option<(u32, u32)>) -> Result<Vec<u8>, Error> {
    let (width, height) = size_hint.unwrap_or((1, 1));
    let object = spa::pod::object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        spa::pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        spa::pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(
            FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::BGRA,
            VideoFormat::RGBx,
            VideoFormat::RGBA,
        ),
        spa::pod::property!(
            FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            Rectangle { width, height },
            Rectangle {
                width: 1,
                height: 1
            },
            Rectangle {
                width: MAX_SIZE,
                height: MAX_SIZE
            }
        ),
        spa::pod::property!(
            FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            Fraction { num: 0, denom: 1 },
            Fraction { num: 0, denom: 1 },
            Fraction {
                num: 1000,
                denom: 1
            }
        ),
    );
    PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(object))
        .map(|(cursor, _)| cursor.into_inner())
        .map_err(|error| Error::Portal(format!("{error:?}")))
}

fn portal_error(error: impl Display) -> Error {
    Error::Portal(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipewire_byte_orders_map_to_shm_formats() {
        assert_eq!(
            shm_format(VideoFormat::BGRx),
            Some(wl_shm::Format::Xrgb8888)
        );
        assert_eq!(
            shm_format(VideoFormat::BGRA),
            Some(wl_shm::Format::Argb8888)
        );
        assert_eq!(
            shm_format(VideoFormat::RGBx),
            Some(wl_shm::Format::Xbgr8888)
        );
        assert_eq!(
            shm_format(VideoFormat::RGBA),
            Some(wl_shm::Format::Abgr8888)
        );
        assert_eq!(shm_format(VideoFormat::I420), None);
    }

    #[test]
    fn the_format_request_serializes() {
        let params = format_params(Some((1920, 1080))).unwrap();
        assert!(Pod::from_bytes(&params).is_some());
        assert!(format_params(None).is_ok());
    }

    #[test]
    fn a_failure_ends_the_wait() {
        let shared = Shared::default();
        shared.fail("gone".into());
        assert!(matches!(
            shared.wait(None, None, |latest| latest.frame.take()),
            Err(Error::Portal(reason)) if reason == "gone"
        ));
    }

    #[test]
    fn a_stopped_flag_ends_the_wait() {
        let shared = Shared::default();
        let stop = AtomicBool::new(true);
        assert!(matches!(
            shared.wait(Some(&stop), None, |latest| latest.frame.take()),
            Err(Error::Stopped)
        ));
    }

    #[test]
    fn the_newest_frame_replaces_an_unread_one() {
        let shared = Shared::default();
        for width in [1, 2] {
            shared.update(|latest| {
                latest.frame = Some(Frame::new(
                    width,
                    1,
                    vec![0; 4 * width as usize],
                    Instant::now(),
                ))
            });
        }
        let frame = shared
            .wait(None, None, |latest| latest.frame.take())
            .unwrap();
        assert_eq!(frame.width, 2);
    }

    #[test]
    fn a_missing_format_times_out() {
        let shared = Shared::default();
        let past = Instant::now();
        assert!(matches!(
            shared.wait(None, Some(past), |latest| latest.size),
            Err(Error::Portal(_))
        ));
    }
}
