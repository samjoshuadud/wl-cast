use std::{
    fs::File,
    os::{fd::AsFd, unix::fs::FileExt},
    time::Instant,
};

use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, WEnum,
    protocol::{wl_buffer::WlBuffer, wl_output::Transform, wl_shm},
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
    ext_image_copy_capture_manager_v1,
    ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
};

use super::{FORMATS, FORMATS_ERROR, State, orient, oriented_size, to_bgrx};
use crate::{Error, Frame};

pub(super) struct ExtCapture {
    queue: EventQueue<State>,
    state: State,
    session: ExtImageCopyCaptureSessionV1,
    buffer: WlBuffer,
    file: File,
    format: wl_shm::Format,
    width: u32,
    height: u32,
    size: (u32, u32),
}

impl ExtCapture {
    pub(super) fn new(
        mut queue: EventQueue<State>,
        mut state: State,
        output_index: usize,
    ) -> Result<Self, Error> {
        let qh = queue.handle();

        let output = state
            .outputs
            .get(output_index)
            .ok_or(Error::OutputNotFound(output_index))?;
        let source_manager = state.ext_source_manager.as_ref().ok_or(Error::Unsupported(
            "ext_output_image_capture_source_manager_v1",
        ))?;
        let capture_manager = state
            .ext_capture_manager
            .as_ref()
            .ok_or(Error::Unsupported("ext_image_copy_capture_manager_v1"))?;

        let source = source_manager.create_source(output, &qh, ());
        let session = capture_manager.create_session(
            &source,
            ext_image_copy_capture_manager_v1::Options::PaintCursors,
            &qh,
            (),
        );

        while !state.ext_constraints_done {
            queue.blocking_dispatch(&mut state)?;
        }

        let format = FORMATS
            .into_iter()
            .find(|format| state.ext_formats.contains(format))
            .ok_or(Error::Unsupported(FORMATS_ERROR))?;

        let width = state.ext_width;
        let height = state.ext_height;
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

        let output_transform = state
            .output_transforms
            .get(output_index)
            .copied()
            .unwrap_or(Transform::Normal);
        let size = oriented_size(width, height, output_transform);

        Ok(Self {
            queue,
            state,
            session,
            buffer,
            file,
            format,
            width,
            height,
            size,
        })
    }

    pub(super) fn size(&self) -> (u32, u32) {
        self.size
    }

    pub(super) fn next_frame(&mut self) -> Result<Frame, Error> {
        self.state.frame_ready = false;
        self.state.frame_failed = false;
        self.state.ext_transform = None;

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

        let transform = self.state.ext_transform.unwrap_or(Transform::Normal);
        if oriented_size(self.width, self.height, transform) != self.size {
            return Err(Error::CaptureFailed);
        }

        let mut data = vec![0; (self.width * self.height * 4) as usize];
        self.file.read_exact_at(&mut data, 0)?;

        Ok(Frame {
            width: self.size.0,
            height: self.size.1,
            data: orient(
                to_bgrx(data, self.format),
                self.width,
                self.height,
                transform,
            ),
            captured_at: Instant::now(),
        })
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
                state.ext_width = width;
                state.ext_height = height;
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat {
                format: WEnum::Value(format),
            } => state.ext_formats.push(format),
            ext_image_copy_capture_session_v1::Event::Done => state.ext_constraints_done = true,
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
            ext_image_copy_capture_frame_v1::Event::Transform {
                transform: WEnum::Value(transform),
            } => state.ext_transform = Some(transform),
            ext_image_copy_capture_frame_v1::Event::Ready => state.frame_ready = true,
            ext_image_copy_capture_frame_v1::Event::Failed { .. } => state.frame_failed = true,
            _ => {}
        }
    }
}
