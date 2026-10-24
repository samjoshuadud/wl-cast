use std::{fmt, time::Instant};

/// A captured screen image.
///
/// Pixels are BGRX: 4 bytes each, blue, green, red, unused.
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
        let frame = Frame {
            width: 2,
            height: 1,
            data: vec![7; 8],
            captured_at: Instant::now(),
        };
        let debug = format!("{frame:?}");
        assert!(debug.contains("width: 2"));
        assert!(debug.contains("data_len: 8"));
        assert!(!debug.contains("7, 7"));
    }
}
