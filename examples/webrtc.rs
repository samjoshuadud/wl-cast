use std::{
    error::Error,
    io::{BufRead, BufReader, ErrorKind, Read, Write},
    net::{IpAddr, TcpListener, TcpStream, UdpSocket},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
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
use wl_cast::{Capture, Encoder, Frame, Input, MouseButton};

const PAGE: &str = include_str!("webrtc.html");

struct Encoded {
    data: Vec<u8>,
    encoded_at: Instant,
}

fn main() -> Result<(), Box<dyn Error>> {
    let ip = local_ip()?;
    let listener = TcpListener::bind("0.0.0.0:8080")?;
    println!("open http://{ip}:8080 in a browser");

    let (rtc, socket) = loop {
        let (stream, _) = listener.accept()?;
        match handle_http(stream, ip) {
            Ok(Some(connection)) => break connection,
            Ok(None) => {}
            Err(e) => eprintln!("http request failed: {e}"),
        }
    };

    let keyframe = Arc::new(AtomicBool::new(true));
    let video = start_video(keyframe.clone());
    run(rtc, socket, video, keyframe)
}

fn handle_http(stream: TcpStream, ip: IpAddr) -> Result<Option<(Rtc, UdpSocket)>, Box<dyn Error>> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;

    let mut content_length = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line.trim().is_empty() {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = value.trim().parse()?;
        }
    }

    if !request_line.starts_with("POST /offer") {
        respond(&stream, "text/html", PAGE)?;
        return Ok(None);
    }

    let mut body = vec![0; content_length];
    reader.read_exact(&mut body)?;
    let offer = SdpOffer::from_sdp_string(std::str::from_utf8(&body)?)?;

    let mut rtc = Rtc::builder().build(Instant::now());
    let socket = UdpSocket::bind((ip, 8080))?;
    rtc.add_local_candidate(Candidate::host(socket.local_addr()?, "udp")?);
    let answer = rtc.sdp_api().accept_offer(offer)?;

    respond(&stream, "application/sdp", &answer.to_sdp_string())?;
    Ok(Some((rtc, socket)))
}

fn respond(mut stream: &TcpStream, content_type: &str, body: &str) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn start_video(keyframe: Arc<AtomicBool>) -> Receiver<Encoded> {
    let (frame_tx, frame_rx) = mpsc::channel();
    let (encoded_tx, encoded_rx) = mpsc::channel();

    thread::spawn(move || {
        if let Err(e) = capture_loop(frame_tx) {
            eprintln!("capture stopped: {e}");
        }
    });
    thread::spawn(move || {
        if let Err(e) = encode_loop(frame_rx, encoded_tx, keyframe) {
            eprintln!("encoder stopped: {e}");
        }
    });

    encoded_rx
}

fn capture_loop(frames: Sender<Frame>) -> Result<(), wl_cast::Error> {
    let mut capture = Capture::new(0)?;
    while frames.send(capture.next_frame()?).is_ok() {}
    Ok(())
}

fn encode_loop(
    frames: Receiver<Frame>,
    encoded: Sender<Encoded>,
    keyframe: Arc<AtomicBool>,
) -> Result<(), wl_cast::Error> {
    let mut encoder: Option<Encoder> = None;
    let mut last_frame: Option<Frame> = None;

    loop {
        match frames.recv_timeout(Duration::from_millis(100)) {
            Ok(frame) => last_frame = Some(frame),
            Err(RecvTimeoutError::Timeout) if keyframe.load(Ordering::Relaxed) => {}
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        let Some(frame) = &last_frame else {
            continue;
        };

        if encoder.is_none() {
            let new_encoder = Encoder::new(frame.width, frame.height)?;
            println!("encoder: {}", new_encoder.name());
            encoder = Some(new_encoder);
        }
        let Some(encoder) = encoder.as_mut() else {
            continue;
        };

        if keyframe.swap(false, Ordering::Relaxed) {
            encoder.request_keyframe();
        }
        let data = encoder.encode(frame)?;
        let message = Encoded {
            data,
            encoded_at: Instant::now(),
        };
        if encoded.send(message).is_err() {
            return Ok(());
        }
    }
}

fn run(
    mut rtc: Rtc,
    socket: UdpSocket,
    video: Receiver<Encoded>,
    keyframe: Arc<AtomicBool>,
) -> Result<(), Box<dyn Error>> {
    let start = Instant::now();
    let mut input = Input::new(0)?;
    let mut video_mid: Option<Mid> = None;
    let mut connected = false;
    let mut buffer = vec![0; 2000];

    loop {
        let timeout = match rtc.poll_output()? {
            Output::Timeout(timeout) => timeout,
            Output::Transmit(transmit) => {
                socket.send_to(&transmit.contents, transmit.destination)?;
                continue;
            }
            Output::Event(event) => {
                match event {
                    Event::IceConnectionStateChange(IceConnectionState::Disconnected) => {
                        println!("viewer disconnected");
                        return Ok(());
                    }
                    Event::Connected => {
                        println!("viewer connected");
                        connected = true;
                        keyframe.store(true, Ordering::Relaxed);
                    }
                    Event::MediaAdded(media) if media.kind == MediaKind::Video => {
                        video_mid = Some(media.mid);
                    }
                    Event::KeyframeRequest(_) => keyframe.store(true, Ordering::Relaxed),
                    Event::ChannelData(data) => {
                        if let Ok(message) = std::str::from_utf8(&data.data) {
                            apply_input(&mut input, message)?;
                        }
                    }
                    _ => {}
                }
                continue;
            }
        };

        while let Ok(encoded) = video.try_recv() {
            if let (true, Some(mid)) = (connected, video_mid) {
                send_video(&mut rtc, mid, encoded, start)?;
            }
        }

        let wait = timeout
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(5));
        if wait.is_zero() {
            rtc.handle_input(RtcInput::Timeout(Instant::now()))?;
            continue;
        }

        socket.set_read_timeout(Some(wait))?;
        match socket.recv_from(&mut buffer) {
            Ok((length, source)) => {
                let receive = Receive {
                    proto: Protocol::Udp,
                    source,
                    destination: socket.local_addr()?,
                    contents: buffer[..length].try_into()?,
                };
                rtc.handle_input(RtcInput::Receive(Instant::now(), receive))?;
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                rtc.handle_input(RtcInput::Timeout(Instant::now()))?;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

fn send_video(
    rtc: &mut Rtc,
    mid: Mid,
    encoded: Encoded,
    start: Instant,
) -> Result<(), Box<dyn Error>> {
    let writer = rtc.writer(mid).ok_or("no video writer")?;
    let pt = writer
        .payload_params()
        .find(|params| {
            let spec = params.spec();
            spec.codec == Codec::H264 && spec.format.packetization_mode == Some(1)
        })
        .map(|params| params.pt())
        .ok_or("browser did not offer H.264")?;

    let seconds = encoded
        .encoded_at
        .saturating_duration_since(start)
        .as_secs_f64();
    let rtp_time = MediaTime::from_90khz((seconds * 90_000.0) as u64);
    writer.write(pt, encoded.encoded_at, rtp_time, encoded.data)?;
    Ok(())
}

fn apply_input(input: &mut Input, message: &str) -> Result<(), wl_cast::Error> {
    let parts: Vec<&str> = message.split_whitespace().collect();
    match parts.as_slice() {
        ["m", x, y] => {
            if let (Ok(x), Ok(y)) = (x.parse(), y.parse()) {
                input.move_to(x, y)?;
            }
        }
        ["b", button, state] => {
            let button = match *button {
                "0" => MouseButton::Left,
                "1" => MouseButton::Middle,
                "2" => MouseButton::Right,
                _ => return Ok(()),
            };
            input.button(button, *state == "1")?;
        }
        ["w", dx, dy] => {
            if let (Ok(dx), Ok(dy)) = (dx.parse(), dy.parse()) {
                input.scroll(dx, dy)?;
            }
        }
        ["k", code, state] => {
            if let Ok(code) = code.parse() {
                input.key(code, *state == "1")?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn local_ip() -> std::io::Result<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect("8.8.8.8:80")?;
    Ok(socket.local_addr()?.ip())
}
