//! Control playback from a phone's browser on the same network.
//!
//! A small HTTP server serves one page and a JSON API. While Ferritone runs it listens on this
//! computer only, where the page opens without a token at http://localhost:47123. When the user
//! turns the phone remote on it also listens on the network, and requests from other devices must
//! carry the secret token from the link shown as a QR code. Nothing is sent to the internet.

use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket},
    path::{Path, PathBuf},
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
/// Step-by-step help for making `ferritone` open the page in a browser.
const SETUP_PAGE: &str = include_str!("remote_setup.html");
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
    Pause,
    PlayQueue(usize),
    PlayPlaylist(usize),
    /// A track by its index in the library list.
    PlayLibrary(usize),
    /// Continues on the computer where the phone stopped.
    PlayQueueAt(Position),
    PlayLibraryAt(Position),
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct Position {
    pub index: usize,
    pub position_ms: u64,
}

#[derive(Clone, Default, PartialEq, Serialize)]
pub struct Entry {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
}

#[derive(Clone, Default, PartialEq, Serialize)]
pub struct LibraryEntry {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
}

/// The whole library, sent to the phone only when it asks, since it can be large.
#[derive(Default)]
pub struct LibraryList {
    pub version: u64,
    pub entries: Vec<LibraryEntry>,
    pub paths: Vec<PathBuf>,
}

#[derive(Clone, Default, PartialEq, Serialize)]
pub struct PlaylistEntry {
    pub name: String,
    pub tracks: usize,
}

/// The colors of Ferritone's theme and accent, so the pages look like the app.
#[derive(Clone, Default, PartialEq, Serialize)]
pub struct Palette {
    pub dark: bool,
    pub background: String,
    pub surface: String,
    pub raised: String,
    pub text: String,
    pub muted: String,
    pub line: String,
    pub accent: String,
    pub accent_text: String,
}

/// What the phone shows; the GTK thread keeps it current.
#[derive(Clone, Default, Serialize)]
pub struct Snapshot {
    pub language: String,
    pub palette: Palette,
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
    /// Changes whenever the library list changes, so the page knows to fetch it again.
    pub library_version: u64,
    pub queue: Vec<Entry>,
    pub playlists: Vec<PlaylistEntry>,
}

/// State shared between the GTK thread and the server threads.
#[derive(Default)]
pub struct Shared {
    pub snapshot: Mutex<Snapshot>,
    /// The files behind `Snapshot::queue`, for streaming to the phone.
    pub queue_paths: Mutex<Vec<PathBuf>>,
    /// The computer's current track, kept current on every poll so its cover never lags
    /// behind a queue that just changed.
    pub current_path: Mutex<Option<PathBuf>>,
    pub library: Mutex<LibraryList>,
}

pub enum Body {
    Bytes(Vec<u8>),
    /// Part of a file, streamed rather than read into memory.
    File {
        path: PathBuf,
        start: u64,
        end: u64,
        total: u64,
    },
}

pub struct Response {
    pub status: &'static str,
    pub content_type: &'static str,
    pub body: Body,
    /// Lets the browser keep the response; only for URLs that name their content.
    pub cacheable: bool,
}

impl Response {
    fn new(status: &'static str, content_type: &'static str, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            content_type,
            body: Body::Bytes(body.into()),
            cacheable: false,
        }
    }

    fn json(value: &impl Serialize) -> Self {
        Self::new(
            "200 OK",
            "application/json",
            serde_json::to_vec(value).unwrap_or_default(),
        )
    }

    fn text(status: &'static str, body: &str) -> Self {
        Self::new(status, "text/plain; charset=utf-8", body)
    }
}

fn param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then_some(value)
    })
}

/// The file a request points at: a queue or library entry by index, never a path from the phone.
fn requested_file(query: &str, shared: &Shared) -> Option<PathBuf> {
    if param(query, "src")? == "current" {
        return shared.current_path.lock().ok()?.clone();
    }
    let index: usize = param(query, "i")?.parse().ok()?;
    match param(query, "src")? {
        "queue" => shared.queue_paths.lock().ok()?.get(index).cloned(),
        "library" => shared.library.lock().ok()?.paths.get(index).cloned(),
        _ => None,
    }
}

fn audio_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mp3") => "audio/mpeg",
        Some("flac") => "audio/flac",
        Some("ogg" | "oga" | "opus") => "audio/ogg",
        Some("wav") => "audio/wav",
        Some("aac") => "audio/aac",
        Some("m4a" | "alac") => "audio/mp4",
        Some("aiff") => "audio/aiff",
        _ => "application/octet-stream",
    }
}

/// Parses a single `bytes=` range against a file of `total` bytes into inclusive offsets.
pub fn parse_range(header: &str, total: u64) -> Option<(u64, u64)> {
    let spec = header.trim().strip_prefix("bytes=")?;
    if spec.contains(',') || total == 0 {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    let (start, end) = if start.is_empty() {
        let suffix: u64 = end.parse().ok()?;
        (total.saturating_sub(suffix), total - 1)
    } else {
        let start: u64 = start.parse().ok()?;
        let end = if end.is_empty() {
            total - 1
        } else {
            end.parse::<u64>().ok()?.min(total - 1)
        };
        (start, end)
    };
    (start <= end && start < total).then_some((start, end))
}

fn stream_file(path: PathBuf, range: Option<&str>) -> Response {
    let Ok(total) = fs::metadata(&path).map(|metadata| metadata.len()) else {
        return Response::text("404 Not Found", "Missing file");
    };
    let content_type = audio_type(&path);
    let (status, start, end) = match range {
        Some(range) => match parse_range(range, total) {
            Some((start, end)) => ("206 Partial Content", start, end),
            None => return Response::text("416 Range Not Satisfiable", "Bad range"),
        },
        None => ("200 OK", 0, total.saturating_sub(1)),
    };
    Response {
        status,
        content_type,
        body: Body::File {
            path,
            start,
            end,
            total,
        },
        cacheable: false,
    }
}

fn image_type(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG") {
        "image/png"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else {
        "image/jpeg"
    }
}

/// One parsed HTTP request.
#[derive(Default)]
pub struct Request<'a> {
    pub method: &'a str,
    pub target: &'a str,
    pub range: Option<&'a str>,
    pub content_type: Option<&'a str>,
    pub body: &'a [u8],
    /// Sent by this computer's own browser from the page itself or a typed address,
    /// so it needs no token.
    pub local: bool,
}

/// Names this computer's browser may use for the page without a token. Checking the Host header
/// stops other websites from reaching the server through DNS rebinding.
fn local_host(host: &str) -> bool {
    let name = match host.rsplit_once(':') {
        Some((name, port)) if !name.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => host,
    };
    matches!(
        name.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "[::1]" | "ferritone"
    )
}

/// Whether a request comes from this computer's browser and not from another website.
pub fn trusted_local(
    peer_is_loopback: bool,
    host: Option<&str>,
    origin: Option<&str>,
    fetch_site: Option<&str>,
) -> bool {
    let Some(host) = host.filter(|host| peer_is_loopback && local_host(host)) else {
        return false;
    };
    // Browsers mark requests that other sites trigger; only typed addresses and the page's own
    // requests are trusted.
    let same_site = matches!(fetch_site, None | Some("none" | "same-origin"));
    let same_origin = origin.is_none_or(|origin| {
        origin
            .strip_prefix("http://")
            .is_some_and(|origin_host| origin_host.eq_ignore_ascii_case(host))
    });
    same_site && same_origin
}

/// Answers one request. Kept free of socket I/O so it can be tested.
pub fn respond(
    request: &Request,
    token: &str,
    shared: &Shared,
    commands: &mpsc::Sender<Command>,
) -> Response {
    let (path, query) = request
        .target
        .split_once('?')
        .unwrap_or((request.target, ""));
    let supplied = param(query, "t").unwrap_or("");
    if !request.local && !tokens_match(supplied, token) {
        return Response::text(
            "403 Forbidden",
            "Scan the QR code in Ferritone again to get a new link.",
        );
    }
    let range = request.range;
    match (request.method, path) {
        ("GET", "/") => Response::new("200 OK", "text/html; charset=utf-8", PAGE),
        ("GET", "/setup") => Response::new("200 OK", "text/html; charset=utf-8", SETUP_PAGE),
        ("GET", "/api/state") => match shared.snapshot.lock() {
            Ok(snapshot) => Response::json(&*snapshot),
            Err(_) => Response::text("500 Internal Server Error", "Unavailable"),
        },
        ("GET", "/api/library") => match shared.library.lock() {
            Ok(library) => Response::json(&serde_json::json!({
                "version": library.version,
                "tracks": library.entries,
            })),
            Err(_) => Response::text("500 Internal Server Error", "Unavailable"),
        },
        ("GET", "/api/audio") => match requested_file(query, shared) {
            Some(path) => stream_file(path, range),
            None => Response::text("404 Not Found", "No such track"),
        },
        ("GET", "/api/cover") => {
            match requested_file(query, shared).and_then(|path| crate::metadata::read_cover(&path))
            {
                // The page puts the track's title, artist, and album in the cover URL, so a URL
                // always means the same image and the browser may keep it.
                Some(image) => Response {
                    cacheable: param(query, "k").is_some(),
                    ..Response::new("200 OK", image_type(&image), image)
                },
                None => Response::text("404 Not Found", "No cover"),
            }
        }
        // Websites cannot send JSON to another site without the browser asking first,
        // so requiring it blocks forged commands.
        ("POST", "/api/command")
            if !request
                .content_type
                .is_some_and(|value| value.starts_with("application/json")) =>
        {
            Response::text("415 Unsupported Media Type", "Send JSON")
        }
        ("POST", "/api/command") => match serde_json::from_slice::<Command>(request.body) {
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
    /// Whether other devices on the network can connect, not only this computer.
    lan: bool,
    stop: Arc<AtomicBool>,
}

impl Server {
    pub fn start(
        token: String,
        shared: Arc<Shared>,
        commands: mpsc::Sender<Command>,
        lan: bool,
    ) -> io::Result<Self> {
        let address = if lan {
            Ipv4Addr::UNSPECIFIED
        } else {
            Ipv4Addr::LOCALHOST
        };
        let listener = TcpListener::bind((address, PREFERRED_PORT))
            .or_else(|_| TcpListener::bind((address, 0)))?;
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
        Ok(Self { port, lan, stop })
    }

    pub fn lan(&self) -> bool {
        self.lan
    }

    pub fn url(&self, ip: IpAddr, token: &str) -> String {
        format!("http://{}/?t={token}", SocketAddr::new(ip, self.port))
    }

    /// The address for this computer's own browser, which needs no token.
    pub fn local_url(&self) -> String {
        format!("http://localhost:{}/", self.port)
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
    // Long enough for a phone that pauses reading while its audio buffer is full.
    stream.set_write_timeout(Some(Duration::from_secs(120)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    let mut header_bytes = reader.read_line(&mut request_line)?;
    let mut parts = request_line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Ok(());
    };
    let peer_is_loopback = stream
        .peer_addr()
        .is_ok_and(|address| address.ip().is_loopback());
    let (mut length, mut range, mut host, mut origin, mut fetch_site, mut content_type) =
        (0, None, None, None, None, None);
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
            } else if name.eq_ignore_ascii_case("range") {
                range = Some(value.trim().to_owned());
            } else if name.eq_ignore_ascii_case("host") {
                host = Some(value.trim().to_owned());
            } else if name.eq_ignore_ascii_case("origin") {
                origin = Some(value.trim().to_owned());
            } else if name.eq_ignore_ascii_case("sec-fetch-site") {
                fetch_site = Some(value.trim().to_ascii_lowercase());
            } else if name.eq_ignore_ascii_case("content-type") {
                content_type = Some(value.trim().to_ascii_lowercase());
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
        respond(
            &Request {
                method,
                target,
                range: range.as_deref(),
                content_type: content_type.as_deref(),
                body: &body,
                local: trusted_local(
                    peer_is_loopback,
                    host.as_deref(),
                    origin.as_deref(),
                    fetch_site.as_deref(),
                ),
            },
            token,
            shared,
            commands,
        ),
    )
}

fn write_response(mut stream: TcpStream, response: Response) -> io::Result<()> {
    let (length, extra) = match &response.body {
        Body::Bytes(bytes) => (bytes.len() as u64, String::new()),
        Body::File {
            start, end, total, ..
        } => (
            end - start + 1,
            if response.status.starts_with("206") {
                format!("Accept-Ranges: bytes\r\nContent-Range: bytes {start}-{end}/{total}\r\n")
            } else {
                "Accept-Ranges: bytes\r\n".to_owned()
            },
        ),
    };
    write!(
        stream,
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {length}\r\n{extra}Cache-Control: {}\r\n\
         X-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        response.status,
        response.content_type,
        if response.cacheable {
            "private, max-age=86400"
        } else {
            "no-store"
        },
    )?;
    match response.body {
        Body::Bytes(bytes) => stream.write_all(&bytes)?,
        Body::File {
            path, start, end, ..
        } => {
            let mut file = fs::File::open(path)?;
            file.seek(SeekFrom::Start(start))?;
            io::copy(&mut file.take(end - start + 1), &mut stream)?;
        }
    }
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

    fn bytes(response: &Response) -> &[u8] {
        match &response.body {
            Body::Bytes(bytes) => bytes,
            Body::File { .. } => panic!("expected bytes"),
        }
    }

    fn call_with(
        shared: &Shared,
        method: &str,
        target: &str,
        range: Option<&str>,
        body: &str,
    ) -> (Response, mpsc::Receiver<Command>) {
        let (sender, receiver) = mpsc::channel();
        let request = Request {
            method,
            target,
            range,
            content_type: Some("application/json"),
            body: body.as_bytes(),
            local: false,
        };
        let response = respond(&request, "secret", shared, &sender);
        (response, receiver)
    }

    fn call(method: &str, target: &str, body: &str) -> (Response, mpsc::Receiver<Command>) {
        let shared = Shared::default();
        shared.snapshot.lock().unwrap().title = "Undertow".into();
        call_with(&shared, method, target, None, body)
    }

    #[test]
    fn requests_without_the_token_are_refused() {
        for target in [
            "/",
            "/?t=wrong",
            "/api/state?t=secre",
            "/api/state",
            "/api/audio?src=queue&i=0",
            "/api/library?t=",
        ] {
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
        assert!(String::from_utf8_lossy(bytes(&page)).contains("<html"));
        let (state, _) = call("GET", "/api/state?_=1&t=secret", "");
        let json: serde_json::Value = serde_json::from_slice(bytes(&state)).unwrap();
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
            (
                r#"{"action":"play_library","value":7}"#,
                Command::PlayLibrary(7),
            ),
            (
                r#"{"action":"play_queue_at","value":{"index":3,"position_ms":4500}}"#,
                Command::PlayQueueAt(Position {
                    index: 3,
                    position_ms: 4500,
                }),
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
    fn ranges_follow_http_rules() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=900-", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=990-2000", 1000), Some((990, 999)));
        for bad in [
            "bytes=1000-",
            "bytes=5-2",
            "items=0-1",
            "bytes=0-1,4-5",
            "bytes=x-",
        ] {
            assert_eq!(parse_range(bad, 1000), None, "{bad}");
        }
    }

    #[test]
    fn audio_is_served_by_index_only() {
        let directory = tempfile::tempdir().unwrap();
        let track = directory.path().join("song.flac");
        fs::write(&track, vec![7u8; 1000]).unwrap();
        let shared = Shared::default();
        *shared.queue_paths.lock().unwrap() = vec![track.clone()];
        shared.library.lock().unwrap().paths = vec![track];

        let (whole, _) = call_with(
            &shared,
            "GET",
            "/api/audio?t=secret&src=queue&i=0",
            None,
            "",
        );
        assert_eq!((whole.status, whole.content_type), ("200 OK", "audio/flac"));
        let (part, _) = call_with(
            &shared,
            "GET",
            "/api/audio?t=secret&src=library&i=0",
            Some("bytes=100-"),
            "",
        );
        assert_eq!(part.status, "206 Partial Content");
        assert!(matches!(
            part.body,
            Body::File {
                start: 100,
                end: 999,
                total: 1000,
                ..
            }
        ));
        for target in [
            "/api/audio?t=secret&src=queue&i=1",
            "/api/audio?t=secret&src=/etc/passwd&i=0",
            "/api/audio?t=secret&i=0",
        ] {
            let (response, _) = call_with(&shared, "GET", target, None, "");
            assert_eq!(response.status, "404 Not Found", "{target}");
        }
    }

    #[test]
    fn the_library_is_listed_on_request() {
        let shared = Shared::default();
        {
            let mut library = shared.library.lock().unwrap();
            library.version = 4;
            library.entries = vec![LibraryEntry {
                title: "Patina".into(),
                artist: "Ines Varga".into(),
                album: "Copper Hours".into(),
                duration_ms: 201_000,
            }];
        }
        let (response, _) = call_with(&shared, "GET", "/api/library?t=secret", None, "");
        let json: serde_json::Value = serde_json::from_slice(bytes(&response)).unwrap();
        assert_eq!(json["version"], 4);
        assert_eq!(json["tracks"][0]["album"], "Copper Hours");
    }

    #[test]
    fn the_server_streams_partial_audio_over_http() {
        let directory = tempfile::tempdir().unwrap();
        let track = directory.path().join("song.mp3");
        fs::write(&track, (0..=255u8).collect::<Vec<_>>()).unwrap();
        let shared = Arc::new(Shared::default());
        *shared.queue_paths.lock().unwrap() = vec![track];
        let (sender, receiver) = mpsc::channel();
        let server = Server::start("abc".into(), shared, sender, false).unwrap();

        let mut stream = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
        write!(
            stream,
            "GET /api/audio?t=abc&src=queue&i=0 HTTP/1.1\r\nRange: bytes=250-\r\n\r\n"
        )
        .unwrap();
        let mut reply = Vec::new();
        stream.read_to_end(&mut reply).unwrap();
        let text = String::from_utf8_lossy(&reply);
        assert!(text.starts_with("HTTP/1.1 206"), "{text}");
        assert!(text.contains("Content-Range: bytes 250-255/256"));
        assert!(reply.ends_with(&[250, 251, 252, 253, 254, 255]));

        let mut stream = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
        let body = r#"{"action":"next"}"#;
        write!(
            stream,
            "POST /api/command?t=abc HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
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
    fn only_this_computers_browser_skips_the_token() {
        let local = |peer, host, origin, site| trusted_local(peer, host, origin, site);
        assert!(local(true, Some("localhost:47123"), None, None));
        assert!(local(true, Some("ferritone:47123"), None, Some("none")));
        assert!(local(
            true,
            Some("127.0.0.1:47123"),
            Some("http://127.0.0.1:47123"),
            Some("same-origin")
        ));
        // Another device on the network.
        assert!(!local(false, Some("localhost:47123"), None, None));
        // A website that rebinds its own name to 127.0.0.1.
        assert!(!local(true, Some("evil.example:47123"), None, None));
        // A website that sends requests to localhost from the browser.
        assert!(!local(
            true,
            Some("localhost:47123"),
            Some("https://evil.example"),
            Some("cross-site")
        ));
        assert!(!local(
            true,
            Some("localhost:47123"),
            None,
            Some("cross-site")
        ));
        assert!(!local(true, None, None, None));
    }

    #[test]
    fn commands_must_be_json_even_from_this_computer() {
        let shared = Shared::default();
        let (sender, receiver) = mpsc::channel();
        let forged = Request {
            method: "POST",
            target: "/api/command",
            content_type: Some("text/plain"),
            body: br#"{"action":"next"}"#,
            local: true,
            ..Request::default()
        };
        assert_eq!(
            respond(&forged, "secret", &shared, &sender).status,
            "415 Unsupported Media Type"
        );
        assert!(receiver.try_recv().is_err());
        let page = Request {
            content_type: Some("application/json"),
            ..forged
        };
        assert_eq!(
            respond(&page, "secret", &shared, &sender).status,
            "204 No Content"
        );
        assert_eq!(receiver.try_recv().unwrap(), Command::Next);
    }

    #[test]
    fn the_server_listens_only_on_this_computer_until_the_remote_is_on() {
        let shared = Arc::new(Shared::default());
        let (sender, _receiver) = mpsc::channel();
        let server = Server::start("abc".into(), shared, sender, false).unwrap();
        assert!(!server.lan());
        let mut stream = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
        write!(
            stream,
            "GET / HTTP/1.1\r\nHost: localhost:{}\r\n\r\n",
            server.port
        )
        .unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
        if let Some(ip) = local_ip() {
            assert!(TcpStream::connect((ip, server.port)).is_err());
        }
    }

    #[test]
    fn tokens_are_long_and_different() {
        let (first, second) = (new_token(), new_token());
        assert_eq!(first.len(), 32);
        assert_ne!(first, second);
    }
}
