#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to connect to the Wayland compositor")]
    Connect(#[from] wayland_client::ConnectError),
    #[error("Wayland connection error")]
    Dispatch(#[from] wayland_client::DispatchError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("compositor does not support {0}")]
    Unsupported(&'static str),
    #[error("no output at index {0}")]
    OutputNotFound(usize),
    #[error("frame capture failed")]
    CaptureFailed,
    #[error("H.264 encoding failed: {0}")]
    Encode(String),
}
