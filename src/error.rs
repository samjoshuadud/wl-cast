/// Errors returned by wl-cast.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Could not connect to the Wayland compositor.
    #[error("failed to connect to the Wayland compositor")]
    Connect(#[from] wayland_client::ConnectError),
    /// The Wayland connection failed.
    #[error("Wayland connection error")]
    Dispatch(#[from] wayland_client::DispatchError),
    /// An I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The compositor or system lacks something wl-cast needs.
    #[error("compositor does not support {0}")]
    Unsupported(&'static str),
    /// No monitor exists at the given index.
    #[error("no output at index {0}")]
    OutputNotFound(usize),
    /// The compositor could not capture the frame.
    #[error("frame capture failed")]
    CaptureFailed,
    /// The frame size does not match the encoder.
    #[error("frame size does not match the encoder")]
    InvalidFrame,
    /// The H.264 encoder failed.
    #[error("H.264 encoding failed: {0}")]
    Encode(String),
    /// A network address could not be resolved.
    #[error("could not resolve address")]
    InvalidAddress,
    /// A WebRTC session failed.
    #[cfg(feature = "webrtc")]
    #[error("WebRTC error: {0}")]
    WebRtc(String),
}
