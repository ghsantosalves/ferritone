//! Submits listens to ListenBrainz (https://listenbrainz.org) through `curl`.
//!
//! The user token reaches curl through its standard input as a config file, so it never
//! appears in the process list.

use std::{
    io::Write,
    process::{Command, Stdio},
};

use serde::{Deserialize, Serialize};

const API: &str = "https://api.listenbrainz.org/1";
/// ListenBrainz accepts up to 1000 listens per request; smaller batches fail less often.
pub const BATCH: usize = 100;
/// Listens kept while offline; the oldest are dropped beyond this.
pub const MAX_PENDING: usize = 1000;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Listen {
    /// When the track started, in seconds since the Unix epoch.
    pub listened_at: u64,
    pub artist: String,
    pub title: String,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

#[derive(Debug, PartialEq)]
pub enum Error {
    /// ListenBrainz refused the token; retrying will not help.
    Token,
    /// No network, a timeout, or a server error; worth retrying later.
    Unavailable,
}

#[derive(Clone, Copy)]
pub enum Kind {
    Single,
    Import,
    PlayingNow,
}

fn track_metadata(listen: &Listen) -> serde_json::Value {
    let mut info = serde_json::json!({
        "media_player": "Ferritone",
        "submission_client": "Ferritone",
        "submission_client_version": env!("CARGO_PKG_VERSION"),
    });
    if let Some(duration) = listen.duration_ms {
        info["duration_ms"] = duration.into();
    }
    let mut metadata = serde_json::json!({
        "artist_name": listen.artist,
        "track_name": listen.title,
        "additional_info": info,
    });
    if let Some(album) = &listen.album {
        metadata["release_name"] = album.as_str().into();
    }
    metadata
}

/// The JSON body of a submit-listens request.
pub fn submit_body(kind: Kind, listens: &[Listen]) -> String {
    let (listen_type, timed) = match kind {
        Kind::Single => ("single", true),
        Kind::Import => ("import", true),
        Kind::PlayingNow => ("playing_now", false),
    };
    let payload: Vec<_> = listens
        .iter()
        .map(|listen| {
            let mut item = serde_json::json!({ "track_metadata": track_metadata(listen) });
            if timed {
                item["listened_at"] = listen.listened_at.into();
            }
            item
        })
        .collect();
    serde_json::json!({ "listen_type": listen_type, "payload": payload }).to_string()
}

/// Quotes a value for a curl config file.
fn quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

fn curl_config(token: &str, url: &str, body: Option<&str>) -> String {
    let mut config = format!(
        "url = {}\nheader = {}\n",
        quote(url),
        quote(&format!("Authorization: Token {}", token.trim()))
    );
    if let Some(body) = body {
        config.push_str("header = \"Content-Type: application/json\"\n");
        config.push_str(&format!("data-binary = {}\n", quote(body)));
    }
    config
}

fn request(token: &str, url: &str, body: Option<&str>) -> Result<(u16, String), Error> {
    let mut child = Command::new("curl")
        .args([
            "-q",
            "--silent",
            "--max-time",
            "15",
            "--max-filesize",
            "1048576",
            "--write-out",
            "\n%{http_code}",
            "--user-agent",
            concat!(
                "Ferritone/",
                env!("CARGO_PKG_VERSION"),
                " (https://github.com/ghsantosalves/ferritone)"
            ),
            "--config",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Error::Unavailable)?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(curl_config(token, url, body).as_bytes())
            .map_err(|_| Error::Unavailable)?;
    }
    let output = child.wait_with_output().map_err(|_| Error::Unavailable)?;
    let output = String::from_utf8(output.stdout).map_err(|_| Error::Unavailable)?;
    let (body, status) = output.rsplit_once('\n').ok_or(Error::Unavailable)?;
    let status = status.trim().parse().map_err(|_| Error::Unavailable)?;
    Ok((status, body.to_owned()))
}

fn check(status: u16) -> Result<(), Error> {
    match status {
        200 => Ok(()),
        401 => Err(Error::Token),
        _ => Err(Error::Unavailable),
    }
}

pub fn submit(token: &str, kind: Kind, listens: &[Listen]) -> Result<(), Error> {
    let body = submit_body(kind, listens);
    let (status, _) = request(token, &format!("{API}/submit-listens"), Some(&body))?;
    check(status)
}

/// The user name the token belongs to.
pub fn validate(token: &str) -> Result<String, Error> {
    let (status, body) = request(token, &format!("{API}/validate-token"), None)?;
    check(status)?;
    let reply: serde_json::Value = serde_json::from_str(&body).map_err(|_| Error::Unavailable)?;
    if reply["valid"].as_bool() == Some(true) {
        Ok(reply["user_name"].as_str().unwrap_or_default().to_owned())
    } else {
        Err(Error::Token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listen() -> Listen {
        Listen {
            listened_at: 1_700_000_000,
            artist: "Coastal \"Static\"".into(),
            title: "Undertow".into(),
            album: Some("Low Tide".into()),
            duration_ms: Some(262_000),
        }
    }

    #[test]
    fn listens_become_the_documented_payload() {
        let body: serde_json::Value =
            serde_json::from_str(&submit_body(Kind::Single, &[listen()])).unwrap();
        assert_eq!(body["listen_type"], "single");
        let item = &body["payload"][0];
        assert_eq!(item["listened_at"], 1_700_000_000);
        assert_eq!(item["track_metadata"]["artist_name"], "Coastal \"Static\"");
        assert_eq!(item["track_metadata"]["release_name"], "Low Tide");
        assert_eq!(
            item["track_metadata"]["additional_info"]["duration_ms"],
            262_000
        );

        let playing: serde_json::Value =
            serde_json::from_str(&submit_body(Kind::PlayingNow, &[listen()])).unwrap();
        assert_eq!(playing["listen_type"], "playing_now");
        assert!(playing["payload"][0].get("listened_at").is_none());
    }

    #[test]
    fn the_curl_config_quotes_the_token_and_body() {
        let body = submit_body(Kind::Single, &[listen()]);
        let config = curl_config(" abc-123 \n", "https://example.org/x", Some(&body));
        assert!(config.contains("header = \"Authorization: Token abc-123\"\n"));
        // Quotes and backslashes in the body are escaped, so the value cannot end early.
        let data = config
            .lines()
            .find(|line| line.starts_with("data-binary"))
            .unwrap();
        assert!(
            data.contains(r#"\"artist_name\":\"Coastal \\\"Static\\\"\""#),
            "{data}"
        );
        assert!(!config.contains("\n\n"));
    }

    #[test]
    fn curl_sends_the_token_header_and_body() {
        use std::io::{BufRead, BufReader, Read};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let (mut headers, mut length) = (Vec::new(), 0);
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
                headers.push(line.trim_end().to_owned());
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let mut stream = stream;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .unwrap();
            (headers, String::from_utf8(body).unwrap())
        });
        let body = submit_body(Kind::Single, &[listen()]);
        let reply = request(
            "secret-token",
            &format!("http://127.0.0.1:{port}/submit"),
            Some(&body),
        );
        assert_eq!(reply, Ok((200, "ok".to_owned())));
        let (headers, received) = server.join().unwrap();
        assert!(
            headers
                .iter()
                .any(|header| header == "Authorization: Token secret-token")
        );
        assert!(
            headers
                .iter()
                .any(|header| header == "Content-Type: application/json")
        );
        assert_eq!(received, body);
    }

    #[test]
    fn statuses_separate_bad_tokens_from_outages() {
        assert_eq!(check(200), Ok(()));
        assert_eq!(check(401), Err(Error::Token));
        assert_eq!(check(503), Err(Error::Unavailable));
    }
}
