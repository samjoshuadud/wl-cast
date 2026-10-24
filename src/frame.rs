use std::time::Instant;

/// A captured screen image.
///
/// Pixels are BGRX: 4 bytes each, blue, green, red, unused.
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
