use std::{
    fmt,
    io::ErrorKind,
    net::{IpAddr, SocketAddr, UdpSocket},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

use str0m::{
    Candidate, Event, IceConnectionState, Input as RtcInput, Output, Rtc,
    change::SdpOffer,
    format::Codec,
    media::{MediaKind, MediaTime, Mid},
    net::{Protocol, Receive},
};

use crate::{Capture, Encoder, Error, Frame, Input, MouseButton};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Settings for a [`RemoteSession`].
#[derive(Debug)]
pub struct SessionConfig {
    /// The monitor to share, counted from 0.
    pub output_index: usize,
    /// This machine's address on the local network.
    pub local_ip: IpAddr,
    /// The UDP port for video and input. Forward it for direct internet connections. 0 picks a free port.
    pub udp_port: u16,
    /// Your public address, if viewers connect from the internet.
    pub public_ip: Option<IpAddr>,
}

/// One viewer watching and controlling the screen over WebRTC.
pub struct RemoteSession {
    rtc: Rtc,
    socket: UdpSocket,
    output_index: usize,
}

struct Encoded {
    data: Vec<u8>,
    encoded_at: Instant,
}

impl RemoteSession {
    /// Takes the browser's SDP offer and returns the session and the SDP answer to send back.
    ///
    /// # Errors
    ///
    /// Returns [`Error::WebRtc`] if the offer is invalid, or [`Error::Io`] if the UDP port cannot be bound.
    pub fn accept(offer: &str, config: &SessionConfig) -> Result<(Self, String), Error> {
        let offer = SdpOffer::from_sdp_string(offer).map_err(webrtc_error)?;
        let mut rtc = Rtc::builder().build(Instant::now());

        let socket = UdpSocket::bind((config.local_ip, config.udp_port))?;
        let local = socket.local_addr()?;
        rtc.add_local_candidate(Candidate::host(local, "udp").map_err(webrtc_error)?);
        if let Some(public_ip) = config.public_ip {
            let public = SocketAddr::new(public_ip, local.port());
            rtc.add_local_candidate(Candidate::host(public, "udp").map_err(webrtc_error)?);
        }

        let answer = rtc
            .sdp_api()
            .accept_offer(offer)
            .map_err(webrtc_error)?
            .to_sdp_string();
        let session = Self {
            rtc,
            socket,
            output_index: config.output_index,
        };
        Ok((session, answer))
    }

    /// Streams the screen and applies the viewer's input until the viewer disconnects.
    ///
    /// # Errors
    ///
    /// Returns [`Error::WebRtc`] if the viewer does not connect within 30 seconds,
    /// or the capture or encoder error if either fails.
    pub fn run(mut self) -> Result<(), Error> {
        let keyframe = Arc::new(AtomicBool::new(true));
        let video = start_video(self.output_index, keyframe.clone());
        let mut input = Input::new(self.output_index)?;

        let start = Instant::now();
        let mut video_mid: Option<Mid> = None;
        let mut connected = false;
        let mut buffer = vec![0; 2000];

        loop {
            if connect_timed_out(connected, start.elapsed()) {
                return Err(webrtc_error("viewer did not connect in time"));
            }

            let timeout = match self.rtc.poll_output().map_err(webrtc_error)? {
                Output::Timeout(timeout) => timeout,
                Output::Transmit(transmit) => {
                    self.socket
                        .send_to(&transmit.contents, transmit.destination)?;
                    continue;
                }
                Output::Event(event) => {
                    match event {
                        Event::IceConnectionStateChange(IceConnectionState::Disconnected) => {
                            return Ok(());
                        }
                        Event::Connected => {
                            connected = true;
                            keyframe.store(true, Ordering::Relaxed);
                        }
                        Event::MediaAdded(media) if media.kind == MediaKind::Video => {
                            video_mid = Some(media.mid);
                        }
                        Event::KeyframeRequest(_) => keyframe.store(true, Ordering::Relaxed),
                        Event::ChannelData(data) => {
                            let event = std::str::from_utf8(&data.data)
                                .ok()
                                .and_then(parse_viewer_event);
                            if let Some(event) = event {
                                apply(&mut input, event)?;
                            }
                        }
                        _ => {}
                    }
                    continue;
                }
            };

            while let Ok(encoded) = video.try_recv() {
                let encoded = encoded?;
                if let (true, Some(mid)) = (connected, video_mid) {
                    send_video(&mut self.rtc, mid, encoded, start)?;
                }
            }

            let wait = timeout
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(5));
            if wait.is_zero() {
                self.rtc
                    .handle_input(RtcInput::Timeout(Instant::now()))
                    .map_err(webrtc_error)?;
                continue;
            }

            self.socket.set_read_timeout(Some(wait))?;
            match self.socket.recv_from(&mut buffer) {
                Ok((length, source)) => {
                    let receive = Receive {
                        proto: Protocol::Udp,
                        source,
                        destination: self.socket.local_addr()?,
                        contents: buffer[..length].try_into().map_err(webrtc_error)?,
                    };
                    self.rtc
                        .handle_input(RtcInput::Receive(Instant::now(), receive))
                        .map_err(webrtc_error)?;
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    self.rtc
                        .handle_input(RtcInput::Timeout(Instant::now()))
                        .map_err(webrtc_error)?;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
}

fn start_video(output_index: usize, keyframe: Arc<AtomicBool>) -> Receiver<Result<Encoded, Error>> {
    let (frame_tx, frame_rx) = mpsc::sync_channel(1);
    let (encoded_tx, encoded_rx) = mpsc::sync_channel(1);

    thread::spawn(move || capture_loop(output_index, frame_tx));
    thread::spawn(move || {
        if let Err(e) = encode_loop(&frame_rx, &encoded_tx, &keyframe) {
            let _ = encoded_tx.send(Err(e));
        }
    });

    encoded_rx
}

fn capture_loop(output_index: usize, frames: SyncSender<Result<Frame, Error>>) {
    let mut capture = match Capture::new(output_index) {
        Ok(capture) => capture,
        Err(e) => {
            let _ = frames.send(Err(e));
            return;
        }
    };
    loop {
        let frame = capture.next_frame();
        let failed = frame.is_err();
        if frames.send(frame).is_err() || failed {
            return;
        }
    }
}

fn encode_loop(
    frames: &Receiver<Result<Frame, Error>>,
    encoded: &SyncSender<Result<Encoded, Error>>,
    keyframe: &AtomicBool,
) -> Result<(), Error> {
    let mut encoder: Option<Encoder> = None;
    let mut last_frame: Option<Frame> = None;

    loop {
        match frames.recv_timeout(Duration::from_millis(100)) {
            Ok(frame) => last_frame = Some(frame?),
            Err(RecvTimeoutError::Timeout) if keyframe.load(Ordering::Relaxed) => {}
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        let Some(frame) = &last_frame else {
            continue;
        };

        if encoder.is_none() {
            encoder = Some(Encoder::new(frame.width, frame.height)?);
        }
        let Some(encoder) = encoder.as_mut() else {
            continue;
        };

        if keyframe.swap(false, Ordering::Relaxed) {
            encoder.request_keyframe();
        }
        let message = Encoded {
            data: encoder.encode(frame)?,
            encoded_at: Instant::now(),
        };
        if encoded.send(Ok(message)).is_err() {
            return Ok(());
        }
    }
}

fn send_video(rtc: &mut Rtc, mid: Mid, encoded: Encoded, start: Instant) -> Result<(), Error> {
    let writer = rtc
        .writer(mid)
        .ok_or_else(|| webrtc_error("no video writer"))?;
    let pt = writer
        .payload_params()
        .find(|params| {
            let spec = params.spec();
            spec.codec == Codec::H264 && spec.format.packetization_mode == Some(1)
        })
        .map(|params| params.pt())
        .ok_or(Error::Unsupported("H.264 in the viewer"))?;

    let seconds = encoded
        .encoded_at
        .saturating_duration_since(start)
        .as_secs_f64();
    let rtp_time = MediaTime::from_90khz((seconds * 90_000.0) as u64);
    writer
        .write(pt, encoded.encoded_at, rtp_time, encoded.data)
        .map_err(webrtc_error)
}

impl fmt::Debug for RemoteSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteSession")
            .field("output_index", &self.output_index)
            .field("local_address", &self.socket.local_addr().ok())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, PartialEq)]
enum ViewerEvent {
    Move { x: f64, y: f64 },
    Button { button: MouseButton, pressed: bool },
    Scroll { dx: f64, dy: f64 },
    Key { code: u32, pressed: bool },
}

fn parse_viewer_event(message: &str) -> Option<ViewerEvent> {
    let parts: Vec<&str> = message.split_whitespace().collect();
    match parts.as_slice() {
        ["m", x, y] => Some(ViewerEvent::Move {
            x: finite(x)?,
            y: finite(y)?,
        }),
        ["b", button, state] => {
            let button = match *button {
                "0" => MouseButton::Left,
                "1" => MouseButton::Middle,
                "2" => MouseButton::Right,
                _ => return None,
            };
            Some(ViewerEvent::Button {
                button,
                pressed: pressed(state)?,
            })
        }
        ["w", dx, dy] => Some(ViewerEvent::Scroll {
            dx: finite(dx)?,
            dy: finite(dy)?,
        }),
        ["k", code, state] => Some(ViewerEvent::Key {
            code: code.parse().ok()?,
            pressed: pressed(state)?,
        }),
        _ => None,
    }
}

fn finite(text: &str) -> Option<f64> {
    text.parse::<f64>().ok().filter(|value| value.is_finite())
}

fn pressed(text: &str) -> Option<bool> {
    match text {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

fn apply(input: &mut Input, event: ViewerEvent) -> Result<(), Error> {
    match event {
        ViewerEvent::Move { x, y } => input.move_to(x, y),
        ViewerEvent::Button { button, pressed } => input.button(button, pressed),
        ViewerEvent::Scroll { dx, dy } => input.scroll(dx, dy),
        ViewerEvent::Key { code, pressed } => input.key(code, pressed),
    }
}

fn connect_timed_out(connected: bool, waited: Duration) -> bool {
    !connected && waited > CONNECT_TIMEOUT
}

fn webrtc_error(error: impl std::fmt::Display) -> Error {
    Error::WebRtc(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use str0m::media::Direction;

    use super::*;

    fn browser_offer() -> String {
        let mut browser = Rtc::builder().build(Instant::now());
        let mut change = browser.sdp_api();
        change.add_media(MediaKind::Video, Direction::RecvOnly, None, None, None);
        change.add_channel("input".into());
        let (offer, _pending) = change.apply().unwrap();
        offer.to_sdp_string()
    }

    fn config(public_ip: Option<IpAddr>) -> SessionConfig {
        SessionConfig {
            output_index: 0,
            local_ip: Ipv4Addr::LOCALHOST.into(),
            udp_port: 0,
            public_ip,
        }
    }

    #[test]
    fn parses_every_viewer_message() {
        assert_eq!(
            parse_viewer_event("m 0.5 0.25"),
            Some(ViewerEvent::Move { x: 0.5, y: 0.25 })
        );
        assert_eq!(
            parse_viewer_event("b 2 1"),
            Some(ViewerEvent::Button {
                button: MouseButton::Right,
                pressed: true
            })
        );
        assert_eq!(
            parse_viewer_event("w 0 -15.5"),
            Some(ViewerEvent::Scroll { dx: 0.0, dy: -15.5 })
        );
        assert_eq!(
            parse_viewer_event("k 30 0"),
            Some(ViewerEvent::Key {
                code: 30,
                pressed: false
            })
        );
    }

    #[test]
    fn rejects_bad_viewer_messages() {
        for message in [
            "", "x", "m 1", "m 1 2 3", "m a b", "m NaN 0", "m inf 0", "b 9 1", "b 0 2", "k -1 1",
            "k 30", "w 1 nan",
        ] {
            assert_eq!(parse_viewer_event(message), None, "{message:?}");
        }
    }

    #[test]
    fn answer_advertises_the_public_address() {
        let public: IpAddr = Ipv4Addr::new(203, 0, 113, 7).into();
        let (_session, answer) =
            RemoteSession::accept(&browser_offer(), &config(Some(public))).unwrap();
        assert!(answer.contains("203.0.113.7"));
        assert!(answer.contains("127.0.0.1"));
    }

    #[test]
    fn answer_has_no_public_address_by_default() {
        let (_session, answer) = RemoteSession::accept(&browser_offer(), &config(None)).unwrap();
        assert!(!answer.contains("203.0.113.7"));
    }

    #[test]
    fn gives_up_only_while_waiting_for_the_viewer() {
        assert!(!connect_timed_out(false, Duration::from_secs(29)));
        assert!(connect_timed_out(false, Duration::from_secs(31)));
        assert!(!connect_timed_out(true, Duration::from_secs(600)));
    }

    #[test]
    fn debug_output_shows_the_monitor_and_local_address() {
        let (session, _answer) = RemoteSession::accept(&browser_offer(), &config(None)).unwrap();
        let debug = format!("{session:?}");
        assert!(debug.contains("output_index: 0"));
        assert!(debug.contains("127.0.0.1"));
    }

    #[test]
    fn rejects_an_invalid_offer() {
        assert!(RemoteSession::accept("not sdp", &config(None)).is_err());
    }
}
