use std::{fmt, time::Instant};

/// A captured screen image.
///
/// Pixels are BGRX: 4 bytes each, blue, green, red, unused.
/// Build one with [`Frame::new`].
#[non_exhaustive]
#[derive(Clone)]
pub struct Frame {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Raw pixels, row by row, `width * height * 4` bytes.
    pub data: Vec<u8>,
    /// When the frame was captured.
    pub captured_at: Instant,
}

impl Frame {
    /// Creates a frame.
    pub fn new(width: u32, height: u32, data: Vec<u8>, captured_at: Instant) -> Self {
        Self {
            width,
            height,
            data,
            captured_at,
        }
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Frame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("data_len", &self.data.len())
            .field("captured_at", &self.captured_at)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_shows_the_data_length_instead_of_the_pixels() {
        let frame = Frame::new(2, 1, vec![7; 8], Instant::now());
        let debug = format!("{frame:?}");
        assert!(debug.contains("width: 2"));
        assert!(debug.contains("data_len: 8"));
        assert!(!debug.contains("7, 7"));
    }
}
