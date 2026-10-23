/// Pixels are BGRX: 4 bytes each, blue, green, red, unused.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}
