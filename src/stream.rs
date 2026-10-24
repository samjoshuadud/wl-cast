use std::{
    fmt,
    net::{SocketAddr, ToSocketAddrs, UdpSocket},
    time::Instant,
};

use crate::Error;

const MAX_PAYLOAD: usize = 1200;
const PAYLOAD_TYPE: u8 = 96;
const CLOCK_RATE: f64 = 90_000.0;
const FU_A: u8 = 28;

/// Sends H.264 video over RTP.
pub struct RtpSender {
    socket: UdpSocket,
    destination: SocketAddr,
    sequence: u16,
    ssrc: u32,
    start: Option<Instant>,
}

impl RtpSender {
    /// Creates a sender for `destination`, for example `192.168.1.5:5004`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidAddress`] if `destination` cannot be resolved.
    pub fn new(destination: impl ToSocketAddrs) -> Result<Self, Error> {
        let destination = destination
            .to_socket_addrs()?
            .next()
            .ok_or(Error::InvalidAddress)?;
        let bind = if destination.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        Ok(Self {
            socket: UdpSocket::bind(bind)?,
            destination,
            sequence: 0,
            ssrc: std::process::id(),
            start: None,
        })
    }

    /// Returns the SDP text a player such as ffplay needs to receive the stream.
    pub fn sdp(&self) -> String {
        let ip = self.destination.ip();
        let family = if ip.is_ipv4() { "IP4" } else { "IP6" };
        let port = self.destination.port();
        format!(
            "v=0\r\n\
             o=- 0 0 IN {family} {ip}\r\n\
             s=wl-cast\r\n\
             c=IN {family} {ip}\r\n\
             t=0 0\r\n\
             m=video {port} RTP/AVP {PAYLOAD_TYPE}\r\n\
             a=rtpmap:{PAYLOAD_TYPE} H264/90000\r\n\
             a=fmtp:{PAYLOAD_TYPE} packetization-mode=1\r\n"
        )
    }

    /// Sends one encoded frame. `captured_at` decides when the player shows it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if a packet cannot be sent.
    pub fn send(&mut self, h264: &[u8], captured_at: Instant) -> Result<(), Error> {
        let start = *self.start.get_or_insert(captured_at);
        let seconds = captured_at.duration_since(start).as_secs_f64();
        let timestamp = (seconds * CLOCK_RATE) as u64 as u32;

        let payloads = payloads(h264);
        let last = payloads.len().saturating_sub(1);

        for (i, payload) in payloads.iter().enumerate() {
            let marker = if i == last { 0x80 } else { 0 };
            let mut packet = Vec::with_capacity(12 + payload.len());
            packet.push(0x80);
            packet.push(marker | PAYLOAD_TYPE);
            packet.extend_from_slice(&self.sequence.to_be_bytes());
            packet.extend_from_slice(&timestamp.to_be_bytes());
            packet.extend_from_slice(&self.ssrc.to_be_bytes());
            packet.extend_from_slice(payload);

            self.socket.send_to(&packet, self.destination)?;
            self.sequence = self.sequence.wrapping_add(1);
        }

        Ok(())
    }
}

impl fmt::Debug for RtpSender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RtpSender")
            .field("destination", &self.destination)
            .field("sequence", &self.sequence)
            .finish_non_exhaustive()
    }
}

fn nal_units(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i..i + 3] == [0, 0, 1] {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }

    starts
        .iter()
        .enumerate()
        .map(|(n, &start)| {
            let end = starts.get(n + 1).map_or(data.len(), |&next| next - 3);
            let mut nal = &data[start..end];
            while let [rest @ .., 0] = nal {
                nal = rest;
            }
            nal
        })
        .filter(|nal| !nal.is_empty())
        .collect()
}

fn payloads(h264: &[u8]) -> Vec<Vec<u8>> {
    let mut payloads = Vec::new();

    for nal in nal_units(h264) {
        if nal.len() <= MAX_PAYLOAD {
            payloads.push(nal.to_vec());
            continue;
        }

        let indicator = (nal[0] & 0xE0) | FU_A;
        let nal_type = nal[0] & 0x1F;
        let chunks: Vec<&[u8]> = nal[1..].chunks(MAX_PAYLOAD - 2).collect();
        let last = chunks.len() - 1;

        for (i, chunk) in chunks.iter().enumerate() {
            let mut header = nal_type;
            if i == 0 {
                header |= 0x80;
            }
            if i == last {
                header |= 0x40;
            }
            let mut payload = vec![indicator, header];
            payload.extend_from_slice(chunk);
            payloads.push(payload);
        }
    }

    payloads
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_nal_units_on_both_start_codes() {
        let data = [
            0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 0, 1, 0x65, 4,
        ];
        let nals = nal_units(&data);
        assert_eq!(nals, vec![&[0x67, 1, 2][..], &[0x68, 3], &[0x65, 4]]);
    }

    #[test]
    fn small_nal_is_sent_as_is() {
        let data = [0, 0, 0, 1, 0x67, 1, 2, 3];
        assert_eq!(payloads(&data), vec![vec![0x67, 1, 2, 3]]);
    }

    #[test]
    fn large_nal_is_fragmented_and_reassembles() {
        let mut nal = vec![0x65];
        nal.extend((0..3000).map(|i| (i % 250 + 1) as u8));
        let mut data = vec![0, 0, 0, 1];
        data.extend(&nal);

        let parts = payloads(&data);
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|p| p.len() <= MAX_PAYLOAD));
        assert_eq!(parts[0][0], 0x60 | FU_A);
        assert_eq!(parts[0][1], 0x80 | 5);
        assert_eq!(parts[1][1], 5);
        assert_eq!(parts[2][1], 0x40 | 5);

        let mut rebuilt = vec![(parts[0][0] & 0xE0) | (parts[0][1] & 0x1F)];
        for part in &parts {
            rebuilt.extend(&part[2..]);
        }
        assert_eq!(rebuilt, nal);
    }

    #[test]
    fn debug_output_shows_the_destination() {
        let sender = RtpSender::new("127.0.0.1:5004").unwrap();
        assert!(format!("{sender:?}").contains("127.0.0.1:5004"));
    }
}
