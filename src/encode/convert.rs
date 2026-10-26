#[cfg(feature = "ffmpeg")]
use yuv::{YuvBiPlanarImageMut, bgra_to_yuv_nv12};
use yuv::{
    YuvChromaSubsampling, YuvConversionMode, YuvPlanarImageMut, YuvRange, YuvStandardMatrix,
    bgra_to_yuv420,
};

use crate::{Error, Frame};

const RANGE: YuvRange = YuvRange::Limited;
const MATRIX: YuvStandardMatrix = YuvStandardMatrix::Bt709;
const MODE: YuvConversionMode = YuvConversionMode::Balanced;
const BYTES_PER_PIXEL: u32 = 4;

pub(super) fn i420_image(width: u32, height: u32) -> YuvPlanarImageMut<'static, u8> {
    YuvPlanarImageMut::alloc(width, height, YuvChromaSubsampling::Yuv420)
}

#[cfg(feature = "ffmpeg")]
pub(super) fn nv12_image(width: u32, height: u32) -> YuvBiPlanarImageMut<'static, u8> {
    YuvBiPlanarImageMut::alloc(width, height, YuvChromaSubsampling::Yuv420)
}

pub(super) fn to_i420(frame: &Frame, image: &mut YuvPlanarImageMut<u8>) -> Result<(), Error> {
    bgra_to_yuv420(
        image,
        &frame.data,
        frame.width * BYTES_PER_PIXEL,
        RANGE,
        MATRIX,
        MODE,
    )
    .map_err(conversion_error)
}

#[cfg(feature = "ffmpeg")]
pub(super) fn to_nv12(frame: &Frame, image: &mut YuvBiPlanarImageMut<u8>) -> Result<(), Error> {
    bgra_to_yuv_nv12(
        image,
        &frame.data,
        frame.width * BYTES_PER_PIXEL,
        RANGE,
        MATRIX,
        MODE,
    )
    .map_err(conversion_error)
}

fn conversion_error(error: yuv::YuvError) -> Error {
    Error::Encode(format!("color conversion failed: {error}"))
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    fn flat(red: u8, green: u8, blue: u8) -> Frame {
        Frame::new(4, 4, [blue, green, red, 0].repeat(16), Instant::now())
    }

    fn close(actual: u8, expected: u8) -> bool {
        actual.abs_diff(expected) <= 1
    }

    const COLORS: [([u8; 3], [u8; 3]); 4] = [
        ([255, 0, 0], [63, 102, 240]),
        ([0, 255, 0], [173, 42, 26]),
        ([0, 0, 255], [32, 240, 118]),
        ([128, 128, 128], [126, 128, 128]),
    ];

    #[test]
    fn colors_become_bt709_limited_range_planes() {
        for ([r, g, b], [y, u, v]) in COLORS {
            let mut image = i420_image(4, 4);
            to_i420(&flat(r, g, b), &mut image).unwrap();
            let planes = (
                image.y_plane.borrow()[0],
                image.u_plane.borrow()[0],
                image.v_plane.borrow()[0],
            );
            assert!(
                close(planes.0, y) && close(planes.1, u) && close(planes.2, v),
                "rgb {r},{g},{b}: got {planes:?}, want {y},{u},{v}"
            );
        }
    }

    #[cfg(feature = "ffmpeg")]
    #[test]
    fn nv12_has_the_same_values_with_chroma_interleaved() {
        for ([r, g, b], [y, u, v]) in COLORS {
            let mut image = nv12_image(4, 4);
            to_nv12(&flat(r, g, b), &mut image).unwrap();
            let planes = (
                image.y_plane.borrow()[0],
                image.uv_plane.borrow()[0],
                image.uv_plane.borrow()[1],
            );
            assert!(
                close(planes.0, y) && close(planes.1, u) && close(planes.2, v),
                "rgb {r},{g},{b}: got {planes:?}, want {y},{u},{v}"
            );
        }
    }

    #[test]
    fn a_frame_of_the_wrong_size_is_an_error_not_a_panic() {
        let mut image = i420_image(8, 8);
        assert!(to_i420(&flat(1, 2, 3), &mut image).is_err());
    }
}
