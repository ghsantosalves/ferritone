//! A small blocking HTTPS client for LRCLIB and ListenBrainz, built on ureq with rustls.
//!
//! Every call blocks until the reply arrives or the timeout passes, so callers run it off the
//! GTK thread (with `gio::spawn_blocking` or on their own thread).

use std::{fmt, io, time::Duration};

/// Replies larger than this are refused.
const MAX_BODY: u64 = 1024 * 1024;

pub const USER_AGENT: &str = concat!(
    "Ferritone/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/ghsantosalves/ferritone)"
);

#[derive(Debug)]
pub enum Error {
    /// The server did not answer within the timeout.
    Timeout,
    /// The name did not resolve, the connection failed, or TLS failed.
    Connection(String),
    /// The reply was larger than `MAX_BODY`.
    TooLarge,
    /// The reply could not be read or was not UTF-8.
    InvalidReply(String),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Timeout => formatter.write_str("the server did not answer in time"),
            Self::Connection(detail) => write!(formatter, "could not connect: {detail}"),
            Self::TooLarge => formatter.write_str("the reply was too large"),
            Self::InvalidReply(detail) => write!(formatter, "unreadable reply: {detail}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<ureq::Error> for Error {
    fn from(error: ureq::Error) -> Self {
        match error {
            ureq::Error::Timeout(_) => Self::Timeout,
            ureq::Error::BodyExceedsLimit(_) => Self::TooLarge,
            ureq::Error::Io(error) if error.kind() == io::ErrorKind::TimedOut => Self::Timeout,
            ureq::Error::Io(error) if error.kind() == io::ErrorKind::InvalidData => {
                Self::InvalidReply(error.to_string())
            }
            ureq::Error::Protocol(_) => Self::InvalidReply(error.to_string()),
            other => Self::Connection(other.to_string()),
        }
    }
}

/// A reply of any status; callers decide what each status means.
#[derive(Debug, PartialEq)]
pub struct Reply {
    pub status: u16,
    pub body: String,
}

/// A client whose whole request, from connecting to the last byte, ends within `timeout`.
pub struct Client {
    agent: ureq::Agent,
}

impl Client {
    pub fn new(timeout: Duration) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .timeout_connect(Some(timeout.min(Duration::from_secs(5))))
            // 4xx and 5xx replies come back as statuses, not errors.
            .http_status_as_error(false)
            .user_agent(USER_AGENT)
            .build()
            .into();
        Self { agent }
    }

    /// GETs `url` with `query` added, URL-encoded.
    pub fn get(
        &self,
        url: &str,
        query: &[(&str, &str)],
        headers: &[(&str, &str)],
    ) -> Result<Reply, Error> {
        let mut request = self.agent.get(url).query_pairs(query.iter().copied());
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        read(request.call()?)
    }

    /// POSTs a JSON body to `url`.
    pub fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Result<Reply, Error> {
        let mut request = self
            .agent
            .post(url)
            .header("Content-Type", "application/json");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        read(request.send(body)?)
    }
}

fn read(mut response: ureq::http::Response<ureq::Body>) -> Result<Reply, Error> {
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_BODY)
        .read_to_string()?;
    Ok(Reply { status, body })
}

#[cfg(test)]
pub mod tests {
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        thread::JoinHandle,
    };

    use super::*;

    /// What a one-shot test server received: the request line, headers, and body.
    pub struct Received {
        pub request_line: String,
        pub headers: Vec<String>,
        pub body: String,
    }

    /// Serves one request on localhost with `reply` (a full HTTP response), or with nothing
    /// for `delay` when `reply` is empty.
    pub fn serve_once(
        reply: &'static [u8],
        delay: Duration,
    ) -> (String, JoinHandle<Option<Received>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().ok()?;
            let mut reader = BufReader::new(stream.try_clone().ok()?);
            let mut request_line = String::new();
            reader.read_line(&mut request_line).ok()?;
            let (mut headers, mut length) = (Vec::new(), 0);
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).ok()?;
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().ok()?;
                }
                headers.push(line.trim_end().to_owned());
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).ok()?;
            std::thread::sleep(delay);
            let mut stream = stream;
            let _ = stream.write_all(reply);
            Some(Received {
                request_line: request_line.trim_end().to_owned(),
                headers,
                body: String::from_utf8(body).ok()?,
            })
        });
        (address, server)
    }

    #[test]
    fn replies_of_any_status_come_back_with_their_body() {
        let (address, server) = serve_once(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 7\r\n\r\nmissing",
            Duration::ZERO,
        );
        let reply = Client::new(Duration::from_secs(5))
            .get(&format!("{address}/x"), &[("q", "a b&c")], &[])
            .unwrap();
        assert_eq!(reply.status, 404);
        assert_eq!(reply.body, "missing");
        let received = server.join().unwrap().unwrap();
        assert_eq!(received.request_line, "GET /x?q=a%20b%26c HTTP/1.1");
        assert!(
            received
                .headers
                .iter()
                .any(|header| header == &format!("user-agent: {USER_AGENT}")
                    || header == &format!("User-Agent: {USER_AGENT}"))
        );
    }

    #[test]
    fn a_silent_server_times_out() {
        let (address, _server) = serve_once(b"", Duration::from_secs(3));
        let error = Client::new(Duration::from_millis(300))
            .get(&address, &[], &[])
            .unwrap_err();
        assert!(matches!(error, Error::Timeout), "{error:?}");
    }

    #[test]
    fn oversized_replies_are_refused() {
        static BIG: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
        let reply = BIG.get_or_init(|| {
            let body = vec![b'x'; MAX_BODY as usize + 1];
            let mut reply =
                format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
            reply.extend(body);
            reply
        });
        let (address, _server) = serve_once(reply, Duration::ZERO);
        let error = Client::new(Duration::from_secs(5))
            .get(&address, &[], &[])
            .unwrap_err();
        assert!(matches!(error, Error::TooLarge), "{error:?}");
    }

    #[test]
    fn unreachable_servers_are_connection_errors() {
        // Nothing listens on a port that was just freed.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let error = Client::new(Duration::from_secs(5))
            .get(&format!("http://127.0.0.1:{port}/"), &[], &[])
            .unwrap_err();
        assert!(matches!(error, Error::Connection(_)), "{error:?}");
    }
}
