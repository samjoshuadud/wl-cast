use std::{
    error::Error,
    io::{BufRead, BufReader, Read, Write},
    net::{IpAddr, TcpListener, TcpStream, UdpSocket},
    time::Duration,
};

use wl_cast::{AccessToken, RemoteSession, SessionConfig};

const PAGE: &str = include_str!("webrtc.html");
const PORT: u16 = 8080;

fn main() -> Result<(), Box<dyn Error>> {
    let public_ip = std::env::args()
        .nth(1)
        .map(|arg| arg.parse::<IpAddr>())
        .transpose()?;
    let local_ip = local_ip()?;
    let config = SessionConfig {
        output_index: 0,
        local_ip,
        udp_port: PORT,
        public_ip,
    };

    let listener = TcpListener::bind(("0.0.0.0", PORT))?;
    let token = AccessToken::generate()?;
    println!(
        "open http://{local_ip}:{PORT}/?t={} in a browser",
        token.as_str()
    );
    if let Some(public_ip) = public_ip {
        println!(
            "from the internet: http://{public_ip}:{PORT}/?t={}",
            token.as_str()
        );
    }

    let session = loop {
        let (stream, _) = listener.accept()?;
        match handle_http(stream, &token, &config) {
            Ok(Some(session)) => break session,
            Ok(None) => {}
            Err(e) => eprintln!("http request failed: {e}"),
        }
    };

    session.run()?;
    Ok(())
}

fn handle_http(
    stream: TcpStream,
    token: &AccessToken,
    config: &SessionConfig,
) -> Result<Option<RemoteSession>, Box<dyn Error>> {
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

    let target = request_line.split_whitespace().nth(1).unwrap_or("");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let given = query.split('&').find_map(|pair| pair.strip_prefix("t="));
    if !given.is_some_and(|given| token.verify(given)) {
        respond(&stream, "403 Forbidden", "text/plain", "forbidden")?;
        return Ok(None);
    }

    if !(request_line.starts_with("POST") && path == "/offer") {
        respond(&stream, "200 OK", "text/html", PAGE)?;
        return Ok(None);
    }

    let mut body = vec![0; content_length];
    reader.read_exact(&mut body)?;
    let (session, answer) = RemoteSession::accept(std::str::from_utf8(&body)?, config)?;

    respond(&stream, "200 OK", "application/sdp", &answer)?;
    Ok(Some(session))
}

fn respond(
    mut stream: &TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn local_ip() -> std::io::Result<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect("8.8.8.8:80")?;
    Ok(socket.local_addr()?.ip())
}
