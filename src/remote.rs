//! Control playback from a phone's browser on the same network.
//!
//! A small HTTP server serves one page and a JSON API. Every request must carry the secret token
//! from the link shown as a QR code, so other devices on the network cannot control the player.
//! The server runs only while the user has turned remote control on.

use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use serde::{Deserialize, Serialize};

/// Tried first so that a bookmarked link keeps working; any free port is used if it is taken.
pub const PREFERRED_PORT: u16 = 47_123;
const PAGE: &str = include_str!("remote.html");
const MAX_HEADER: usize = 8 * 1024;
const MAX_BODY: usize = 4 * 1024;

/// A request from the phone, applied on the GTK thread.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "action", content = "value", rename_all = "snake_case")]
pub enum Command {
    Toggle,
    Next,
    Previous,
    /// Position in milliseconds.
    Seek(u64),
    /// Volume from 0 to 1.
    Volume(f64),
    Shuffle,
    Repeat,
    PlayQueue(usize),
    PlayPlaylist(usize),
}

#[derive(Clone, Default, PartialEq, Serialize)]
pub struct Entry {
    pub title: String,
    pub artist: String,
}

#[derive(Clone, Default, PartialEq, Serialize)]
pub struct PlaylistEntry {
    pub name: String,
    pub tracks: usize,
}

/// What the phone shows; the GTK thread keeps it current.
#[derive(Clone, Default, Serialize)]
pub struct Snapshot {
    pub language: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub playing: bool,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub volume: f64,
    pub shuffle: bool,
    pub repeat: String,
    pub current: Option<usize>,
    /// Changes whenever the cover changes, so the page knows to reload it.
    pub cover: u64,
    pub queue: Vec<Entry>,
    pub playlists: Vec<PlaylistEntry>,
}

/// State shared between the GTK thread and the server threads.
#[derive(Default)]
pub struct Shared {
    pub snapshot: Mutex<Snapshot>,
    pub cover: Mutex<Option<Vec<u8>>>,
}

pub struct Response {
    pub status: &'static str,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl Response {
    fn new(status: &'static str, content_type: &'static str, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            content_type,
            body: body.into(),
        }
    }

    fn text(status: &'static str, body: &str) -> Self {
        Self::new(status, "text/plain; charset=utf-8", body)
    }
}

/// Answers one request. Kept free of I/O so it can be tested.
pub fn respond(
    method: &str,
    target: &str,
    body: &[u8],
    token: &str,
    shared: &Shared,
    commands: &mpsc::Sender<Command>,
) -> Response {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let supplied = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("t="))
        .unwrap_or("");
    if !tokens_match(supplied, token) {
        return Response::text(
            "403 Forbidden",
            "Scan the QR code in Ferritone again to get a new link.",
        );
    }
    match (method, path) {
        ("GET", "/") => Response::new("200 OK", "text/html; charset=utf-8", PAGE),
        ("GET", "/api/state") => match shared.snapshot.lock() {
            Ok(snapshot) => Response::new(
                "200 OK",
                "application/json",
                serde_json::to_vec(&*snapshot).unwrap_or_default(),
            ),
            Err(_) => Response::text("500 Internal Server Error", "Unavailable"),
        },
        ("GET", "/api/cover") => match shared.cover.lock().ok().and_then(|cover| cover.clone()) {
            Some(png) => Response::new("200 OK", "image/png", png),
            None => Response::text("404 Not Found", "No cover"),
        },
        ("POST", "/api/command") => match serde_json::from_slice::<Command>(body) {
            Ok(command) => {
                let _ = commands.send(command);
                Response::new("204 No Content", "text/plain", Vec::new())
            }
            Err(_) => Response::text("400 Bad Request", "Unknown command"),
        },
        _ => Response::text("404 Not Found", "Not found"),
    }
}

/// Compares without stopping at the first difference, so timing does not reveal the token.
fn tokens_match(supplied: &str, token: &str) -> bool {
    supplied.len() == token.len()
        && supplied
            .bytes()
            .zip(token.bytes())
            .fold(0, |difference, (a, b)| difference | (a ^ b))
            == 0
}

/// A new random token for the link.
pub fn new_token() -> String {
    let mut bytes = [0u8; 16];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .is_err()
    {
        for chunk in bytes.chunks_mut(4) {
            chunk.copy_from_slice(&gtk::glib::random_int().to_ne_bytes());
        }
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The address other devices on the network can reach this computer at.
pub fn local_ip() -> Option<IpAddr> {
    // Connecting a UDP socket sends nothing; it only picks the interface of the default route.
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

/// The running server; dropping it stops accepting connections.
pub struct Server {
    port: u16,
    stop: Arc<AtomicBool>,
}

impl Server {
    pub fn start(
        token: String,
        shared: Arc<Shared>,
        commands: mpsc::Sender<Command>,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, PREFERRED_PORT))
            .or_else(|_| TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)))?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let token = Arc::new(token);
        thread::Builder::new()
            .name("remote-control".into())
            .spawn(move || {
                while !stopped.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let (token, shared, commands) =
                                (token.clone(), shared.clone(), commands.clone());
                            thread::spawn(move || {
                                let _ = serve(stream, &token, &shared, &commands);
                            });
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(100));
                        }
                        Err(_) => thread::sleep(Duration::from_millis(500)),
                    }
                }
            })?;
        Ok(Self { port, stop })
    }

    pub fn url(&self, ip: IpAddr, token: &str) -> String {
        format!("http://{}/?t={token}", SocketAddr::new(ip, self.port))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn serve(
    stream: TcpStream,
    token: &str,
    shared: &Shared,
    commands: &mpsc::Sender<Command>,
) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    let mut header_bytes = reader.read_line(&mut request_line)?;
    let mut parts = request_line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Ok(());
    };
    let mut length = 0;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        header_bytes += read;
        if read == 0 || line == "\r\n" || line == "\n" || header_bytes > MAX_HEADER {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    if header_bytes > MAX_HEADER || length > MAX_BODY {
        return write_response(stream, Response::text("413 Content Too Large", "Too large"));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    write_response(
        stream,
        respond(method, target, &body, token, shared, commands),
    )
}

fn write_response(mut stream: TcpStream, response: Response) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        response.status,
        response.content_type,
        response.body.len()
    )?;
    stream.write_all(&response.body)?;
    stream.flush()
}

/// Draws a QR code for `text` into a GTK drawing area.
pub fn qr_area(text: &str, size: i32) -> gtk::DrawingArea {
    use gtk::prelude::*;
    let area = gtk::DrawingArea::new();
    area.set_content_width(size);
    area.set_content_height(size);
    let Ok(code) = qrcode::QrCode::new(text.as_bytes()) else {
        return area;
    };
    let width = code.width();
    let dark: Vec<bool> = code
        .to_colors()
        .into_iter()
        .map(|color| color == qrcode::Color::Dark)
        .collect();
    area.set_draw_func(move |_, context, area_width, area_height| {
        // A white quiet zone of four modules keeps the code readable on a dark theme.
        let modules = width + 8;
        let side = f64::from(area_width.min(area_height));
        let module = (side / modules as f64).floor().max(1.0);
        let offset_x = (f64::from(area_width) - module * modules as f64) / 2.0;
        let offset_y = (f64::from(area_height) - module * modules as f64) / 2.0;
        context.set_source_rgb(1.0, 1.0, 1.0);
        context.rectangle(
            offset_x,
            offset_y,
            module * modules as f64,
            module * modules as f64,
        );
        let _ = context.fill();
        context.set_source_rgb(0.0, 0.0, 0.0);
        for (index, _) in dark.iter().enumerate().filter(|(_, dark)| **dark) {
            let (x, y) = (index % width + 4, index / width + 4);
            context.rectangle(
                offset_x + x as f64 * module,
                offset_y + y as f64 * module,
                module,
                module,
            );
        }
        let _ = context.fill();
    });
    area
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(method: &str, target: &str, body: &str) -> (Response, mpsc::Receiver<Command>) {
        let shared = Shared::default();
        shared.snapshot.lock().unwrap().title = "Undertow".into();
        let (sender, receiver) = mpsc::channel();
        let response = respond(method, target, body.as_bytes(), "secret", &shared, &sender);
        (response, receiver)
    }

    #[test]
    fn requests_without_the_token_are_refused() {
        for target in ["/", "/?t=wrong", "/api/state?t=secre", "/api/state"] {
            assert_eq!(
                call("GET", target, "").0.status,
                "403 Forbidden",
                "{target}"
            );
        }
        let (response, commands) = call("POST", "/api/command", r#"{"action":"next"}"#);
        assert_eq!(response.status, "403 Forbidden");
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn the_page_and_state_are_served_with_the_token() {
        let (page, _) = call("GET", "/?t=secret", "");
        assert_eq!(page.status, "200 OK");
        assert!(String::from_utf8(page.body).unwrap().contains("<html"));
        let (state, _) = call("GET", "/api/state?t=secret&_=1", "");
        let json: serde_json::Value = serde_json::from_slice(&state.body).unwrap();
        assert_eq!(json["title"], "Undertow");
    }

    #[test]
    fn commands_reach_the_player() {
        for (body, expected) in [
            (r#"{"action":"toggle"}"#, Command::Toggle),
            (r#"{"action":"seek","value":61000}"#, Command::Seek(61_000)),
            (r#"{"action":"volume","value":0.5}"#, Command::Volume(0.5)),
            (
                r#"{"action":"play_playlist","value":2}"#,
                Command::PlayPlaylist(2),
            ),
        ] {
            let (response, commands) = call("POST", "/api/command?t=secret", body);
            assert_eq!(response.status, "204 No Content");
            assert_eq!(commands.try_recv().unwrap(), expected);
        }
        let (response, commands) = call("POST", "/api/command?t=secret", r#"{"action":"rm"}"#);
        assert_eq!(response.status, "400 Bad Request");
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn the_server_answers_over_http() {
        let shared = Arc::new(Shared::default());
        let (sender, receiver) = mpsc::channel();
        let server = Server::start("abc".into(), shared, sender).unwrap();
        let mut stream = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
        let body = r#"{"action":"next"}"#;
        write!(
            stream,
            "POST /api/command?t=abc HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.1 204"), "{reply}");
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
            Command::Next
        );
    }

    #[test]
    fn tokens_are_long_and_different() {
        let (first, second) = (new_token(), new_token());
        assert_eq!(first.len(), 32);
        assert_ne!(first, second);
    }
}
