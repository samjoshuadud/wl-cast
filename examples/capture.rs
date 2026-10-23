use std::{
    fs::File,
    os::{fd::AsFd, unix::fs::FileExt},
};

use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum, delegate_noop,
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::connect_to_env()?;
    let mut queue = conn.new_event_queue::<State>();
    let qh = queue.handle();
    let mut state = State::default();

    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut state)?;

    let output = state.outputs.first().ok_or("no outputs")?;
    let source_manager = state
        .source_manager
        .as_ref()
        .ok_or("ext_output_image_capture_source_manager_v1 not supported")?;
    let capture_manager = state
        .capture_manager
        .as_ref()
        .ok_or("ext_image_copy_capture_manager_v1 not supported")?;

    let source = source_manager.create_source(output, &qh, ());
    let session = capture_manager.create_session(
        &source,
        ext_image_copy_capture_manager_v1::Options::empty(),
        &qh,
        (),
    );

    while !state.constraints_done {
        queue.blocking_dispatch(&mut state)?;
    }

    let format = wl_shm::Format::Xrgb8888;
    if !state.formats.contains(&format) {
        return Err("compositor does not offer Xrgb8888".into());
    }

    let width = state.width;
    let height = state.height;
    let stride = width * 4;
    let size = stride * height;

    let file = File::from(memfd_create("wl-cast-frame", MemfdFlags::CLOEXEC)?);
    file.set_len(size as u64)?;

    let shm = state.shm.as_ref().ok_or("wl_shm not supported")?;
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

    let frame = session.create_frame(&qh, ());
    frame.attach_buffer(&buffer);
    frame.damage_buffer(0, 0, width as i32, height as i32);
    frame.capture();

    while !state.frame_ready && !state.frame_failed {
        queue.blocking_dispatch(&mut state)?;
    }
    if state.frame_failed {
        return Err("capture failed".into());
    }

    let mut pixels = vec![0u8; size as usize];
    file.read_exact_at(&mut pixels, 0)?;

    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }

    let path = "captures/frame.png";
    std::fs::create_dir_all("captures")?;
    image::RgbaImage::from_raw(width, height, pixels)
        .ok_or("buffer size mismatch")?
        .save(path)?;
    println!("saved {path} ({width}x{height})");

    Ok(())
}
