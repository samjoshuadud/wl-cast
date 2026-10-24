use std::{
    fs::File,
    os::{fd::AsFd, unix::fs::FileExt},
    time::Instant,
};

use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, WEnum,
    protocol::{
        wl_buffer::WlBuffer,
        wl_output::{Transform, WlOutput},
        wl_shm::Format,
    },
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, Flags, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

use super::{State, orient, oriented_size};
use crate::{Error, Frame};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct BufferInfo {
    format: WEnum<Format>,
    width: u32,
    height: u32,
    stride: u32,
}

pub(super) struct WlrCapture {
    queue: EventQueue<State>,
    state: State,
    manager: ZwlrScreencopyManagerV1,
    output: WlOutput,
    output_index: usize,
    info: BufferInfo,
    file: File,
    buffer: WlBuffer,
}

impl WlrCapture {
    pub(super) fn new(
        mut queue: EventQueue<State>,
        mut state: State,
        output_index: usize,
    ) -> Result<Self, Error> {
        let qh = queue.handle();
        let output = state
            .outputs
            .get(output_index)
            .cloned()
            .ok_or(Error::OutputNotFound(output_index))?;
        let manager = state
            .wlr_manager
            .clone()
            .ok_or(Error::Unsupported("wlr-screencopy"))?;
        let shm = state.shm.clone().ok_or(Error::Unsupported("wl_shm"))?;

        let (probe, info) = request_frame(&mut queue, &mut state, &manager, &output)?;
        probe.destroy();

        let WEnum::Value(format) = info.format else {
            return Err(Error::Unsupported(
                "Xrgb8888 or Argb8888 shared memory buffers",
            ));
        };
        let size = info.stride as usize * info.height as usize;
        let fd =
            memfd_create("wl-cast-frame", MemfdFlags::CLOEXEC).map_err(std::io::Error::from)?;
        let file = File::from(fd);
        file.set_len(size as u64)?;

        let pool = shm.create_pool(file.as_fd(), size as i32, &qh, ());
        let buffer = pool.create_buffer(
            0,
            info.width as i32,
            info.height as i32,
            info.stride as i32,
            format,
            &qh,
            (),
        );

        Ok(Self {
            queue,
            state,
            manager,
            output,
            output_index,
            info,
            file,
            buffer,
        })
    }

    pub(super) fn size(&self) -> (u32, u32) {
        oriented_size(self.info.width, self.info.height, self.transform())
    }

    fn transform(&self) -> Transform {
        self.state
            .output_transforms
            .get(self.output_index)
            .copied()
            .unwrap_or(Transform::Normal)
    }

    pub(super) fn next_frame(&mut self) -> Result<Frame, Error> {
        let (frame, info) = request_frame(
            &mut self.queue,
            &mut self.state,
            &self.manager,
            &self.output,
        )?;
        if info != self.info {
            frame.destroy();
            return Err(Error::CaptureFailed);
        }

        if self.state.wlr_version >= 2 {
            frame.copy_with_damage(&self.buffer);
        } else {
            frame.copy(&self.buffer);
        }
        while !self.state.frame_ready && !self.state.frame_failed {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        frame.destroy();

        if self.state.frame_failed {
            return Err(Error::CaptureFailed);
        }

        let mut raw = vec![0; info.stride as usize * info.height as usize];
        self.file.read_exact_at(&mut raw, 0)?;

        let rows = pack_rows(&raw, info.width, info.stride, self.state.wlr_y_invert);
        let (width, height) = self.size();
        Ok(Frame {
            width,
            height,
            data: orient(rows, info.width, info.height, self.transform()),
            captured_at: Instant::now(),
        })
    }
}

fn request_frame(
    queue: &mut EventQueue<State>,
    state: &mut State,
    manager: &ZwlrScreencopyManagerV1,
    output: &WlOutput,
) -> Result<(ZwlrScreencopyFrameV1, BufferInfo), Error> {
    state.wlr_buffer = None;
    state.wlr_buffer_done = false;
    state.wlr_y_invert = false;
    state.frame_ready = false;
    state.frame_failed = false;

    let frame = manager.capture_output(1, output, &queue.handle(), ());
    loop {
        if state.frame_failed {
            frame.destroy();
            return Err(Error::CaptureFailed);
        }
        if let Some(info) = state.wlr_buffer
            && (state.wlr_version < 3 || state.wlr_buffer_done)
        {
            return match check(info) {
                Ok(()) => Ok((frame, info)),
                Err(error) => {
                    frame.destroy();
                    Err(error)
                }
            };
        }
        queue.blocking_dispatch(state)?;
    }
}

fn check(info: BufferInfo) -> Result<(), Error> {
    if !matches!(
        info.format,
        WEnum::Value(Format::Xrgb8888 | Format::Argb8888)
    ) {
        return Err(Error::Unsupported(
            "Xrgb8888 or Argb8888 shared memory buffers",
        ));
    }
    let row = info.width.checked_mul(4);
    if info.width == 0 || info.height == 0 || row.is_none_or(|row| info.stride < row) {
        return Err(Error::CaptureFailed);
    }
    Ok(())
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

impl Dispatch<ZwlrScreencopyFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        _frame: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                state.wlr_buffer = Some(BufferInfo {
                    format,
                    width,
                    height,
                    stride,
                })
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => state.wlr_buffer_done = true,
            zwlr_screencopy_frame_v1::Event::Flags {
                flags: WEnum::Value(flags),
            } => state.wlr_y_invert = flags.contains(Flags::YInvert),
            zwlr_screencopy_frame_v1::Event::Ready { .. } => state.frame_ready = true,
            zwlr_screencopy_frame_v1::Event::Failed => state.frame_failed = true,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(format: WEnum<Format>, width: u32, height: u32, stride: u32) -> BufferInfo {
        BufferInfo {
            format,
            width,
            height,
            stride,
        }
    }

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
    fn accepts_the_two_common_formats() {
        for format in [Format::Xrgb8888, Format::Argb8888] {
            assert!(check(info(WEnum::Value(format), 4, 4, 16)).is_ok());
        }
    }

    #[test]
    fn rejects_other_formats() {
        let other = info(WEnum::Value(Format::Rgb565), 4, 4, 8);
        assert!(matches!(check(other), Err(Error::Unsupported(_))));
        let unknown = info(WEnum::Unknown(0xdead), 4, 4, 16);
        assert!(matches!(check(unknown), Err(Error::Unsupported(_))));
    }

    #[test]
    fn rejects_sizes_that_cannot_be_right() {
        let format = WEnum::Value(Format::Xrgb8888);
        assert!(matches!(
            check(info(format, 0, 4, 16)),
            Err(Error::CaptureFailed)
        ));
        assert!(matches!(
            check(info(format, 4, 0, 16)),
            Err(Error::CaptureFailed)
        ));
        assert!(matches!(
            check(info(format, 4, 4, 8)),
            Err(Error::CaptureFailed)
        ));
    }
}
