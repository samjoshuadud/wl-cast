/// Errors returned by wl-cast.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Could not connect to the Wayland compositor.
    ///
    /// The string is the underlying reason.
    #[error("failed to connect to the Wayland compositor")]
    Connect(String),
    /// The Wayland connection failed.
    ///
    /// The string is the underlying reason.
    #[error("Wayland connection error")]
    Dispatch(String),
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
    /// A caller stopped capture or the remote session.
    #[error("stopped")]
    Stopped,
    /// The frame size does not match the encoder.
    #[error("frame size does not match the encoder")]
    InvalidFrame,
    /// The H.264 encoder failed.
    #[error("H.264 encoding failed: {0}")]
    Encode(String),
    /// A network address could not be resolved.
    #[error("could not resolve address")]
    InvalidAddress,
    /// The Miracast setup was rejected or the sink sent something unexpected.
    #[error("miracast setup failed: {0}")]
    Wfd(String),
    /// A WebRTC session failed.
    #[cfg(feature = "webrtc")]
    #[error("WebRTC error: {0}")]
    WebRtc(String),
    /// The desktop portal or its PipeWire stream failed, or the user cancelled sharing.
    #[cfg(feature = "portal")]
    #[error("desktop portal error: {0}")]
    Portal(String),
}

impl From<wayland_client::ConnectError> for Error {
    fn from(error: wayland_client::ConnectError) -> Self {
        Error::Connect(error.to_string())
    }
}

impl From<wayland_client::DispatchError> for Error {
    fn from(error: wayland_client::DispatchError) -> Self {
        Error::Dispatch(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_and_dispatch_keep_a_stable_message() {
        let connect = Error::Connect("no display".into());
        assert_eq!(
            connect.to_string(),
            "failed to connect to the Wayland compositor"
        );
        assert!(format!("{connect:?}").contains("no display"));

        let dispatch = Error::Dispatch("connection lost".into());
        assert_eq!(dispatch.to_string(), "Wayland connection error");
        assert!(format!("{dispatch:?}").contains("connection lost"));
    }
}
