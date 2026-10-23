use std::{
    fs::File,
    os::{fd::AsFd, unix::fs::FileExt},
    time::Instant,
};

use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, WEnum, delegate_noop,
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
    image_copy_capture::v1::client::{
        ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
        ext_image_copy_capture_manager_v1::{self, ExtImageCopyCaptureManagerV1},
        ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
    },
};

use crate::{Error, Frame};

/// Captures frames from one output (monitor).
pub struct Capture {
    queue: EventQueue<State>,
    state: State,
    session: ExtImageCopyCaptureSessionV1,
    buffer: WlBuffer,
    file: File,
    width: u32,
    height: u32,
}

impl Capture {
    /// Starts capturing the output at `output_index`, where 0 is the first monitor.
    pub fn new(output_index: usize) -> Result<Self, Error> {
        let conn = Connection::connect_to_env()?;
        let mut queue = conn.new_event_queue::<State>();
        let qh = queue.handle();
        let mut state = State::default();

        conn.display().get_registry(&qh, ());
        queue.roundtrip(&mut state)?;

        let output = state
            .outputs
            .get(output_index)
            .ok_or(Error::OutputNotFound(output_index))?;
        let source_manager = state.source_manager.as_ref().ok_or(Error::Unsupported(
            "ext_output_image_capture_source_manager_v1",
        ))?;
        let capture_manager = state
            .capture_manager
            .as_ref()
            .ok_or(Error::Unsupported("ext_image_copy_capture_manager_v1"))?;

        let source = source_manager.create_source(output, &qh, ());
        let session = capture_manager.create_session(
            &source,
            ext_image_copy_capture_manager_v1::Options::PaintCursors,
            &qh,
            (),
        );

        while !state.constraints_done {
            queue.blocking_dispatch(&mut state)?;
        }

        let format = wl_shm::Format::Xrgb8888;
        if !state.formats.contains(&format) {
            return Err(Error::Unsupported("Xrgb8888 shared memory buffers"));
        }

        let width = state.width;
        let height = state.height;
        let stride = width * 4;
        let size = stride * height;

        let fd =
            memfd_create("wl-cast-frame", MemfdFlags::CLOEXEC).map_err(std::io::Error::from)?;
        let file = File::from(fd);
        file.set_len(size as u64)?;

        let shm = state.shm.as_ref().ok_or(Error::Unsupported("wl_shm"))?;
        let pool = shm.create_pool(file.as_fd(), size as i32, &qh, ());
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride as i32,
            format,
            &qh,
            (),
        );

        Ok(Self {
            queue,
            state,
            session,
            buffer,
            file,
            width,
            height,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Waits for the next frame. Blocks while the screen is idle.
    pub fn next_frame(&mut self) -> Result<Frame, Error> {
        self.state.frame_ready = false;
        self.state.frame_failed = false;

        let frame = self.session.create_frame(&self.queue.handle(), ());
        frame.attach_buffer(&self.buffer);
        frame.damage_buffer(0, 0, self.width as i32, self.height as i32);
        frame.capture();

        while !self.state.frame_ready && !self.state.frame_failed {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        frame.destroy();

        if self.state.frame_failed {
            return Err(Error::CaptureFailed);
        }

        let mut data = vec![0; (self.width * self.height * 4) as usize];
        self.file.read_exact_at(&mut data, 0)?;

        Ok(Frame {
            width: self.width,
            height: self.height,
            data,
            captured_at: Instant::now(),
        })
    }
}

#[derive(Default)]
struct State {
    outputs: Vec<WlOutput>,
    shm: Option<WlShm>,
    source_manager: Option<ExtOutputImageCaptureSourceManagerV1>,
    capture_manager: Option<ExtImageCopyCaptureManagerV1>,
    width: u32,
    height: u32,
    formats: Vec<wl_shm::Format>,
    constraints_done: bool,
    frame_ready: bool,
    frame_failed: bool,
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
            name, interface, ..
        } = event
        {
            match interface.as_str() {
                "wl_output" => state.outputs.push(registry.bind(name, 1, qh, ())),
                "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
                "ext_output_image_capture_source_manager_v1" => {
                    state.source_manager = Some(registry.bind(name, 1, qh, ()))
                }
                "ext_image_copy_capture_manager_v1" => {
                    state.capture_manager = Some(registry.bind(name, 1, qh, ()))
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for State {
    fn event(
        state: &mut Self,
        _session: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                state.width = width;
                state.height = height;
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat {
                format: WEnum::Value(format),
            } => state.formats.push(format),
            ext_image_copy_capture_session_v1::Event::Done => state.constraints_done = true,
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        _frame: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_frame_v1::Event::Ready => state.frame_ready = true,
            ext_image_copy_capture_frame_v1::Event::Failed { .. } => state.frame_failed = true,
            _ => {}
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
