use crate::{Error, Frame};

const SESSION: &str = "00000001";
const VIDEO_PID: u16 = 0x101;
const PMT_PID: u16 = 0x100;
const TS_PACKET: usize = 188;
const RTP_PACKETS: usize = 7;

/// A picture size the sink agreed to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoMode {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Frames per second.
    pub frame_rate: u32,
}

/// One step of a Miracast setup.
#[derive(Debug, PartialEq, Eq)]
pub enum WfdAction {
    /// Write this RTSP message to the sink.
    Send(String),
    /// The sink is ready for video on `port`.
    Ready {
        /// UDP port on the sink.
        port: u16,
        /// Picture size to send.
        video: VideoMode,
    },
    /// The sink asked for a fresh full picture.
    Keyframe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    WaitM1,
    WaitM2,
    WaitM3,
    WaitM4,
    WaitM5,
    WaitSetup,
    WaitPlay,
    Streaming,
}

#[derive(Clone, Copy, Debug)]
struct Mode {
    width: u32,
    height: u32,
    rate: u32,
    bit: u32,
    level: u8,
}

const MODES: &[Mode] = &[
    Mode {
        width: 1280,
        height: 720,
        rate: 30,
        bit: 5,
        level: 0x01,
    },
    Mode {
        width: 1920,
        height: 1080,
        rate: 30,
        bit: 7,
        level: 0x04,
    },
    Mode {
        width: 640,
        height: 480,
        rate: 60,
        bit: 0,
        level: 0x01,
    },
];

/// The Miracast source handshake.
///
/// Feed it bytes from the sink. Write back every [`WfdAction::Send`].
/// [`WfdAction::Ready`] means the sink will play MPEG-TS on that UDP port.
#[derive(Debug)]
pub struct WfdSource {
    host: String,
    buffer: Vec<u8>,
    stage: Stage,
    cseq: u32,
    port: u16,
    server_port: u16,
    last: String,
    video: VideoMode,
}

impl WfdSource {
    /// Creates a source whose presentation URL uses `host`, for example `192.168.49.200`.
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            buffer: Vec::new(),
            stage: Stage::WaitM1,
            cseq: 0,
            port: 0,
            server_port: 0,
            last: String::from("OPTIONS"),
            video: VideoMode {
                width: 1280,
                height: 720,
                frame_rate: 30,
            },
        }
    }

    /// Names the UDP port this source sends from, for the SETUP reply.
    pub fn set_server_port(&mut self, port: u16) {
        self.server_port = port;
    }

    /// Returns the first message to write on the socket the sink opened.
    pub fn start(&mut self) -> String {
        self.cseq = 1;
        self.stage = Stage::WaitM1;
        format!(
            "OPTIONS * RTSP/1.0\r\nCSeq: {}\r\nRequire: org.wfa.wfd1.0\r\n\r\n",
            self.cseq
        )
    }

    /// Handles the next bytes from the sink.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Wfd`] when the sink rejects the setup or the message is not RTSP.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<WfdAction>, Error> {
        self.buffer.extend_from_slice(bytes);
        let mut actions = Vec::new();
        while let Some(message) = take_message(&mut self.buffer)? {
            actions.extend(self.on_message(&message)?);
        }
        Ok(actions)
    }

    fn on_message(&mut self, message: &Message) -> Result<Vec<WfdAction>, Error> {
        if let Some(status) = message.status()
            && status >= 400
        {
            return Err(Error::Wfd(format!(
                "the sink returned {status} to {}",
                self.last
            )));
        }
        if message.method() == Some("PLAY") {
            return self.play(message);
        }
        if message.method() == Some("OPTIONS") {
            return Ok(self.options(message));
        }
        if message.method() == Some("SETUP") {
            return self.setup(message);
        }
        if message.method() == Some("TEARDOWN") {
            return Err(Error::Wfd("the tablet closed the session".into()));
        }
        if message.status() == Some(200) {
            return self.ok(message);
        }
        if self.stage == Stage::Streaming {
            return Ok(self.streaming(message));
        }
        Ok(Vec::new())
    }

    fn options(&mut self, message: &Message) -> Vec<WfdAction> {
        let reply = format!(
            "RTSP/1.0 200 OK\r\nCSeq: {}\r\nPublic: org.wfa.wfd1.0, SETUP, TEARDOWN, PLAY, PAUSE, GET_PARAMETER, SET_PARAMETER\r\n\r\n",
            message.cseq()
        );
        if matches!(self.stage, Stage::WaitM1 | Stage::WaitM2) {
            self.stage = Stage::WaitM3;
            vec![WfdAction::Send(reply), WfdAction::Send(self.ask())]
        } else {
            vec![WfdAction::Send(reply)]
        }
    }

    fn ok(&mut self, message: &Message) -> Result<Vec<WfdAction>, Error> {
        match self.stage {
            Stage::WaitM1 => {
                self.stage = Stage::WaitM2;
                Ok(Vec::new())
            }
            Stage::WaitM3 => {
                if let Some(port) = rtp_port(&message.raw) {
                    self.port = port;
                }
                self.video = choose_mode(&message.raw)?.mode();
                self.stage = Stage::WaitM4;
                Ok(vec![WfdAction::Send(self.choose())])
            }
            Stage::WaitM4 => {
                self.stage = Stage::WaitM5;
                Ok(vec![WfdAction::Send(self.trigger())])
            }
            Stage::WaitM5 => {
                self.stage = Stage::WaitSetup;
                Ok(Vec::new())
            }
            Stage::WaitPlay => {
                self.stage = Stage::Streaming;
                Ok(vec![WfdAction::Ready {
                    port: self.port,
                    video: self.video,
                }])
            }
            _ => Ok(Vec::new()),
        }
    }

    fn setup(&mut self, message: &Message) -> Result<Vec<WfdAction>, Error> {
        if let Some(port) = rtp_port(&message.raw) {
            self.port = port;
        }
        if self.port == 0 {
            return Err(Error::Wfd("the sink did not give a video port".into()));
        }
        let transport = if self.server_port == 0 {
            format!("client_port={}", self.port)
        } else {
            format!(
                "client_port={};server_port={}-{}",
                self.port,
                self.server_port,
                self.server_port.saturating_add(1)
            )
        };
        let reply = format!(
            "RTSP/1.0 200 OK\r\nCSeq: {}\r\nSession: {SESSION};timeout=30\r\nTransport: RTP/AVP/UDP;unicast;{transport}\r\n\r\n",
            message.cseq()
        );
        self.stage = Stage::WaitPlay;
        Ok(vec![WfdAction::Send(reply)])
    }

    fn play(&mut self, message: &Message) -> Result<Vec<WfdAction>, Error> {
        self.stage = Stage::Streaming;
        let reply = format!(
            "RTSP/1.0 200 OK\r\nCSeq: {}\r\nSession: {SESSION}\r\n\r\n",
            message.cseq()
        );
        Ok(vec![
            WfdAction::Send(reply),
            WfdAction::Ready {
                port: self.port,
                video: self.video,
            },
        ])
    }

    fn streaming(&mut self, message: &Message) -> Vec<WfdAction> {
        let reply = format!("RTSP/1.0 200 OK\r\nCSeq: {}\r\n\r\n", message.cseq());
        let mut actions = vec![WfdAction::Send(reply)];
        if message.raw.to_ascii_lowercase().contains("wfd_idr_request") {
            actions.push(WfdAction::Keyframe);
        }
        actions
    }

    fn ask(&mut self) -> String {
        // ponytail: no audio. Send LPCM silence if a sink rejects "none".
        self.request(
            "GET_PARAMETER",
            "rtsp://localhost/wfd1.0",
            "wfd_video_formats\r\nwfd_audio_codecs\r\nwfd_client_rtp_ports\r\n",
        )
    }

    fn choose(&mut self) -> String {
        let bit = MODES
            .iter()
            .find(|mode| mode.width == self.video.width && mode.rate == self.video.frame_rate)
            .map(|mode| mode.bit)
            .unwrap_or(5);
        let level = MODES
            .iter()
            .find(|mode| mode.bit == bit)
            .map(|mode| mode.level)
            .unwrap_or(0x01);
        let cea = 1u32 << bit;
        let body = format!(
            "wfd_video_formats: {bit:02X} 00 01 {level:02X} {cea:08X} 00000000 00000000 00 0000 0000 00 none none\r\nwfd_audio_codecs: none\r\nwfd_presentation_URL: rtsp://{}/wfd1.0/streamid=0 none\r\nwfd_client_rtp_ports: RTP/AVP/UDP;unicast {} 0 mode=play\r\n",
            self.host, self.port
        );
        self.request("SET_PARAMETER", "rtsp://localhost/wfd1.0", &body)
    }

    fn trigger(&mut self) -> String {
        self.request(
            "SET_PARAMETER",
            "rtsp://localhost/wfd1.0",
            "wfd_trigger_method: SETUP\r\n",
        )
    }

    fn request(&mut self, method: &str, url: &str, body: &str) -> String {
        self.cseq += 1;
        self.last = method.to_string();
        if body.is_empty() {
            format!(
                "{method} {url} RTSP/1.0\r\nCSeq: {}\r\nSession: {SESSION}\r\n\r\n",
                self.cseq
            )
        } else {
            format!(
                "{method} {url} RTSP/1.0\r\nCSeq: {}\r\nContent-Type: text/parameters\r\nContent-Length: {}\r\n\r\n{body}",
                self.cseq,
                body.len()
            )
        }
    }
}

impl Mode {
    fn mode(self) -> VideoMode {
        VideoMode {
            width: self.width,
            height: self.height,
            frame_rate: self.rate,
        }
    }
}

struct Message {
    raw: String,
}

impl Message {
    fn status(&self) -> Option<u16> {
        let mut words = self.raw.split_whitespace();
        words
            .next()?
            .starts_with("RTSP/")
            .then(|| words.next()?.parse().ok())?
    }

    fn method(&self) -> Option<&str> {
        if self.status().is_some() {
            return None;
        }
        self.raw.split_whitespace().next()
    }

    fn cseq(&self) -> &str {
        header(&self.raw, "CSeq").unwrap_or("0")
    }
}

fn header<'a>(raw: &'a str, name: &str) -> Option<&'a str> {
    raw.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().trim_end_matches('\r'))
    })
}

fn take_message(buffer: &mut Vec<u8>) -> Result<Option<Message>, Error> {
    let Some(end) = buffer.windows(4).position(|mark| mark == b"\r\n\r\n") else {
        if buffer.len() > 65_536 {
            return Err(Error::Wfd("the rtsp header is too long".into()));
        }
        return Ok(None);
    };
    let header = std::str::from_utf8(&buffer[..end])
        .map_err(|_| Error::Wfd("the tablet sent a non-utf8 message".into()))?;
    let length = content_length(header);
    if length > 1_048_576 {
        return Err(Error::Wfd("the rtsp body is too long".into()));
    }
    let total = end + 4 + length;
    if buffer.len() < total {
        return Ok(None);
    }
    let raw = std::str::from_utf8(&buffer[..total])
        .map_err(|_| Error::Wfd("the tablet sent a non-utf8 message".into()))?
        .to_string();
    buffer.drain(..total);
    Ok(Some(Message { raw }))
}

fn content_length(text: &str) -> usize {
    header(text, "Content-Length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn rtp_port(text: &str) -> Option<u16> {
    if let Some(rest) = text.split("client_port=").nth(1) {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(port) = digits.parse::<u16>()
            && port != 0
        {
            return Some(port);
        }
    }
    let rest = text.split("unicast").nth(1)?.trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let port = digits.parse().ok()?;
    (port != 0).then_some(port)
}

fn choose_mode(body: &str) -> Result<Mode, Error> {
    let Some(bits) = cea_bitmap(body) else {
        return Ok(MODES[0]);
    };
    MODES
        .iter()
        .copied()
        .find(|mode| bits & (1 << mode.bit) != 0)
        .ok_or_else(|| Error::Wfd("the sink has no supported video mode".into()))
}

fn cea_bitmap(body: &str) -> Option<u32> {
    let line = body
        .lines()
        .find(|line| line.to_ascii_lowercase().contains("wfd_video_formats:"))?;
    let mut parts = line.split_whitespace();
    parts.next()?;
    let _native = parts.next()?;
    let _preferred = parts.next()?;
    let _profile = parts.next()?;
    let _level = parts.next()?;
    u32::from_str_radix(parts.next()?, 16).ok()
}

/// Resizes a BGRX frame to `width` by `height`.
///
/// # Errors
///
/// Returns [`Error::InvalidFrame`] when a size is 0 or the frame is shorter than its size.
pub fn scale_bgra(frame: &Frame, width: u32, height: u32) -> Result<Frame, Error> {
    let expected = (frame.width as usize)
        .saturating_mul(frame.height as usize)
        .saturating_mul(4);
    if width == 0
        || height == 0
        || frame.width == 0
        || frame.height == 0
        || frame.data.len() < expected
    {
        return Err(Error::InvalidFrame);
    }
    if frame.width == width && frame.height == height {
        return Ok(frame.clone());
    }
    let mut data = Vec::with_capacity((width as usize) * (height as usize) * 4);
    for y in 0..height {
        let sy = y * frame.height / height;
        for x in 0..width {
            let sx = x * frame.width / width;
            let start = ((sy * frame.width + sx) * 4) as usize;
            data.extend_from_slice(&frame.data[start..start + 4]);
        }
    }
    Ok(Frame::new(width, height, data, frame.captured_at))
}

/// Packs H.264 into MPEG-TS RTP packets a Miracast sink can play.
#[derive(Debug)]
pub struct MpegTsRtp {
    sequence: u16,
    pat_cc: u8,
    pmt_cc: u8,
    video_cc: u8,
}

impl MpegTsRtp {
    /// Creates a packer. Packet numbers start at 0.
    pub fn new() -> Self {
        Self {
            sequence: 0,
            pat_cc: 0,
            pmt_cc: 0,
            video_cc: 0,
        }
    }

    /// Returns one RTP datagram per up to seven transport packets.
    ///
    /// `annex_b` is one picture, including start codes. `pts` is the 90 kHz timestamp.
    pub fn packets(&mut self, annex_b: &[u8], pts: u32) -> Vec<Vec<u8>> {
        if annex_b.is_empty() {
            return Vec::new();
        }
        let mut ts = Vec::new();
        push_psi(&mut ts, 0, &mut self.pat_cc, &pat());
        push_psi(&mut ts, PMT_PID, &mut self.pmt_cc, &pmt());
        push_pes(&mut ts, &mut self.video_cc, u64::from(pts), annex_b);
        let mut out = Vec::new();
        for chunk in ts.chunks(RTP_PACKETS * TS_PACKET) {
            out.push(rtp(self.sequence, pts, chunk));
            self.sequence = self.sequence.wrapping_add(1);
        }
        if let Some(last) = out.last_mut() {
            last[1] |= 0x80;
        }
        out
    }
}

impl Default for MpegTsRtp {
    fn default() -> Self {
        Self::new()
    }
}

fn rtp(sequence: u16, pts: u32, payload: &[u8]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(12 + payload.len());
    packet.push(0x80);
    packet.push(33);
    packet.extend_from_slice(&sequence.to_be_bytes());
    packet.extend_from_slice(&pts.to_be_bytes());
    packet.extend_from_slice(&1u32.to_be_bytes());
    packet.extend_from_slice(payload);
    packet
}

fn push_psi(out: &mut Vec<u8>, pid: u16, cc: &mut u8, section: &[u8]) {
    let mut offset = 0;
    let mut first = true;
    while offset < section.len() {
        let (packet, took) = ts_packet(pid, first, cc, None, &section[offset..]);
        out.extend_from_slice(&packet);
        offset += took;
        first = false;
        if took == 0 {
            break;
        }
    }
}

fn push_pes(out: &mut Vec<u8>, cc: &mut u8, pts: u64, annex_b: &[u8]) {
    let pes = pes(pts, annex_b);
    let mut offset = 0;
    let mut first = true;
    while offset < pes.len() {
        let pcr = first.then_some(pts);
        let (packet, took) = ts_packet(VIDEO_PID, first, cc, pcr, &pes[offset..]);
        out.extend_from_slice(&packet);
        offset += took;
        first = false;
        if took == 0 {
            break;
        }
    }
}

fn pes(pts: u64, payload: &[u8]) -> Vec<u8> {
    let mut pes = vec![0, 0, 1, 0xe0, 0, 0, 0x80, 0x80, 5];
    pes.extend_from_slice(&pts_bytes(pts));
    pes.extend_from_slice(payload);
    pes
}

fn pts_bytes(pts: u64) -> [u8; 5] {
    let pts = pts & 0x1_ffff_ffff;
    [
        0x21 | (((pts >> 29) as u8) & 0x0e),
        (pts >> 22) as u8,
        0x01 | ((((pts >> 14) as u8) & 0x7f) << 1),
        (pts >> 7) as u8,
        0x01 | (((pts as u8) & 0x7f) << 1),
    ]
}

fn pat() -> Vec<u8> {
    let mut section = vec![
        0x00, 0x00, 0xb0, 0x0d, 0x00, 0x01, 0xc1, 0x00, 0x00, 0x00, 0x01, 0xe1, 0x00,
    ];
    let crc = mpeg_crc(&section[1..]);
    section.extend_from_slice(&crc.to_be_bytes());
    section
}

fn pmt() -> Vec<u8> {
    let mut section = vec![
        0x00, 0x02, 0xb0, 0x12, 0x00, 0x01, 0xc1, 0x00, 0x00, 0xe1, 0x01, 0xf0, 0x00, 0x1b, 0xe1,
        0x01, 0xf0, 0x00,
    ];
    let crc = mpeg_crc(&section[1..]);
    section.extend_from_slice(&crc.to_be_bytes());
    section
}

fn mpeg_crc(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc ^= u32::from(byte) << 24;
        for _ in 0..8 {
            if crc & 0x8000_0000 != 0 {
                crc = (crc << 1) ^ 0x04c1_1db7;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

fn ts_packet(
    pid: u16,
    pusi: bool,
    cc: &mut u8,
    pcr: Option<u64>,
    payload: &[u8],
) -> ([u8; TS_PACKET], usize) {
    let mut packet = [0xffu8; TS_PACKET];
    packet[0] = 0x47;
    packet[1] = ((u8::from(pusi)) << 6) | ((pid >> 8) as u8 & 0x1f);
    packet[2] = pid as u8;
    let mut adapt = Vec::new();
    if let Some(pcr) = pcr {
        adapt.push(0x10);
        adapt.extend_from_slice(&pcr_bytes(pcr));
    }
    let min_adapt = if adapt.is_empty() { 0 } else { 1 + adapt.len() };
    let space = TS_PACKET - 4 - min_adapt;
    let take = payload.len().min(space);
    let stuff = space - take;
    packet[3] = if min_adapt == 0 && stuff == 0 {
        0x10
    } else {
        0x30
    } | (*cc & 0x0f);
    if min_adapt == 0 && stuff == 0 {
        packet[4..4 + take].copy_from_slice(&payload[..take]);
    } else if adapt.is_empty() {
        if stuff == 1 {
            packet[4] = 0;
            packet[5..5 + take].copy_from_slice(&payload[..take]);
        } else {
            packet[4] = (stuff - 1) as u8;
            packet[5] = 0;
            let at = 4 + stuff;
            packet[at..at + take].copy_from_slice(&payload[..take]);
        }
    } else {
        packet[4] = (adapt.len() + stuff) as u8;
        packet[5..5 + adapt.len()].copy_from_slice(&adapt);
        let at = 5 + adapt.len() + stuff;
        packet[at..at + take].copy_from_slice(&payload[..take]);
    }
    *cc = cc.wrapping_add(1) & 0x0f;
    (packet, take)
}

fn pcr_bytes(pcr: u64) -> [u8; 6] {
    let base = pcr & 0x1_ffff_ffff;
    [
        (base >> 25) as u8,
        (base >> 17) as u8,
        (base >> 9) as u8,
        (base >> 1) as u8,
        ((base as u8) << 7) | 0x7e,
        0,
    ]
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    fn feed(source: &mut WfdSource, message: &str) -> Vec<WfdAction> {
        source.push(message.as_bytes()).unwrap()
    }

    #[test]
    fn a_sink_reply_ends_ready_to_play() {
        let mut source = WfdSource::new("192.168.49.200");
        let opening = source.start();
        assert!(opening.contains("OPTIONS * RTSP/1.0\r\n"));
        assert!(opening.contains("Require: org.wfa.wfd1.0\r\n"));

        let actions = feed(
            &mut source,
            "RTSP/1.0 200 OK\r\nCSeq: 1\r\nPublic: org.wfa.wfd1.0, GET_PARAMETER, SET_PARAMETER\r\n\r\n",
        );
        assert!(actions.is_empty());

        let actions = feed(
            &mut source,
            "OPTIONS * RTSP/1.0\r\nCSeq: 1\r\nRequire: org.wfa.wfd1.0\r\n\r\n",
        );
        let messages = messages_of(&actions);
        assert!(messages[0].contains("Public: org.wfa.wfd1.0"));
        assert!(messages[1].contains("GET_PARAMETER"));
        assert!(messages[1].contains("wfd_client_rtp_ports\r\n"));

        let body = "wfd_video_formats: 40 00 02 02 0001FFFF 00000000 00000000 00 0000 0000 00 none none\r\nwfd_audio_codecs: LPCM 00000003 00\r\nwfd_client_rtp_ports: RTP/AVP/UDP;unicast 19000 0 mode=play\r\n";
        let response = format!(
            "RTSP/1.0 200 OK\r\nCSeq: 2\r\nContent-Type: text/parameters\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let actions = feed(&mut source, &response);
        let chosen = &messages_of(&actions)[0];
        assert!(chosen.contains("00000020"));
        assert!(chosen.contains("wfd_audio_codecs: none"));
        assert!(chosen.contains("rtsp://192.168.49.200/wfd1.0/streamid=0"));
        assert!(chosen.contains("unicast 19000"));

        assert!(
            messages_of(&feed(&mut source, "RTSP/1.0 200 OK\r\nCSeq: 3\r\n\r\n"))[0]
                .contains("wfd_trigger_method: SETUP")
        );
        assert!(feed(&mut source, "RTSP/1.0 200 OK\r\nCSeq: 4\r\n\r\n").is_empty());

        source.set_server_port(50000);
        let actions = feed(
            &mut source,
            "SETUP rtsp://192.168.49.200/wfd1.0/streamid=0 RTSP/1.0\r\nCSeq: 2\r\nTransport: RTP/AVP/UDP;unicast;client_port=19000\r\n\r\n",
        );
        let messages = messages_of(&actions);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("Session: 00000001"));
        assert!(messages[0].contains("server_port=50000-50001"));

        let actions = feed(
            &mut source,
            "PLAY rtsp://192.168.49.200/wfd1.0/streamid=0 RTSP/1.0\r\nCSeq: 3\r\nSession: 00000001\r\n\r\n",
        );
        assert!(matches!(actions[0], WfdAction::Send(_)));
        assert_eq!(
            actions[1],
            WfdAction::Ready {
                port: 19000,
                video: VideoMode {
                    width: 1280,
                    height: 720,
                    frame_rate: 30,
                },
            }
        );
    }

    #[test]
    fn a_split_header_waits_for_the_rest() {
        let mut source = WfdSource::new("192.168.49.200");
        source.start();
        assert!(
            source
                .push(b"RTSP/1.0 200 OK\r\nCSeq: 1\r\n")
                .unwrap()
                .is_empty()
        );
        assert!(
            source
                .push(b"\r\n\r\nOPTIONS * RTSP/1.0\r\nCSeq: 1\r\n\r\n")
                .unwrap()
                .len()
                >= 2
        );
    }

    #[test]
    fn a_rejected_setup_is_an_error() {
        let mut source = WfdSource::new("192.168.49.200");
        source.start();
        feed(&mut source, "OPTIONS * RTSP/1.0\r\nCSeq: 1\r\n\r\n");
        let error = source
            .push(b"RTSP/1.0 400 Bad Request\r\nCSeq: 2\r\n\r\n")
            .unwrap_err();
        assert!(error.to_string().contains("400"));
    }

    #[test]
    fn a_keepalive_after_play_is_answered() {
        let mut source = playing();
        let actions = feed(
            &mut source,
            "GET_PARAMETER rtsp://192.168.49.200/wfd1.0 RTSP/1.0\r\nCSeq: 6\r\nContent-Length: 0\r\n\r\n",
        );
        assert_eq!(
            actions,
            vec![WfdAction::Send("RTSP/1.0 200 OK\r\nCSeq: 6\r\n\r\n".into())]
        );
        let actions = feed(
            &mut source,
            "SET_PARAMETER rtsp://localhost/wfd1.0 RTSP/1.0\r\nCSeq: 7\r\nContent-Length: 17\r\n\r\nwfd_idr_request\r\n",
        );
        assert!(actions.contains(&WfdAction::Keyframe));
    }

    #[test]
    fn a_picture_is_mpeg_ts_inside_rtp() {
        let mut rtp = MpegTsRtp::new();
        let packets = rtp.packets(&[0, 0, 0, 1, 0x65, 0x88], 3_000);
        assert!(!packets.is_empty());
        assert_eq!(packets[0][0], 0x80);
        assert_eq!(packets.last().unwrap()[1] & 0x7f, 33);
        assert_eq!(packets.last().unwrap()[1] & 0x80, 0x80);
        let payload = &packets[0][12..];
        assert_eq!(payload.len() % TS_PACKET, 0);
        assert!(payload.chunks(TS_PACKET).all(|packet| packet[0] == 0x47));
        assert!(payload.contains(&0x65));
    }

    #[test]
    fn a_smaller_frame_keeps_the_corner_pixel() {
        let frame = Frame::new(
            2,
            2,
            vec![9, 1, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            Instant::now(),
        );
        let scaled = scale_bgra(&frame, 1, 1).unwrap();
        assert_eq!(scaled.data, vec![9, 1, 2, 3]);
        assert!(scale_bgra(&frame, 0, 1).is_err());
    }

    fn messages_of(actions: &[WfdAction]) -> Vec<&str> {
        actions
            .iter()
            .map(|action| match action {
                WfdAction::Send(message) => message.as_str(),
                _ => "",
            })
            .collect()
    }

    fn playing() -> WfdSource {
        let mut source = WfdSource::new("192.168.49.200");
        source.start();
        feed(&mut source, "OPTIONS * RTSP/1.0\r\nCSeq: 1\r\n\r\n");
        let body = "wfd_video_formats: 00 00 01 01 00000020 00000000 00000000 00 0000 0000 00 none none\r\nwfd_client_rtp_ports: RTP/AVP/UDP;unicast 19000 0 mode=play\r\n";
        feed(
            &mut source,
            &format!(
                "RTSP/1.0 200 OK\r\nCSeq: 2\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
        );
        feed(&mut source, "RTSP/1.0 200 OK\r\nCSeq: 3\r\n\r\n");
        feed(&mut source, "RTSP/1.0 200 OK\r\nCSeq: 4\r\n\r\n");
        feed(
            &mut source,
            "SETUP rtsp://192.168.49.200/wfd1.0/streamid=0 RTSP/1.0\r\nCSeq: 1\r\nTransport: RTP/AVP/UDP;unicast;client_port=19000\r\n\r\n",
        );
        feed(
            &mut source,
            "PLAY rtsp://192.168.49.200/wfd1.0/streamid=0 RTSP/1.0\r\nCSeq: 3\r\nSession: 00000001\r\n\r\n",
        );
        source
    }
}
