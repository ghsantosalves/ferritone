//! Submits listens to ListenBrainz (https://listenbrainz.org).
//!
//! The user token travels only in the Authorization header over HTTPS.

use std::{sync::OnceLock, time::Duration};

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

fn request(token: &str, url: &str, body: Option<&str>) -> Result<(u16, String), Error> {
    static CLIENT: OnceLock<crate::http::Client> = OnceLock::new();
    let client = CLIENT.get_or_init(|| crate::http::Client::new(Duration::from_secs(15)));
    let authorization = format!("Token {}", token.trim());
    let headers = [("Authorization", authorization.as_str())];
    let reply = match body {
        Some(body) => client.post_json(url, &headers, body),
        None => client.get(url, &[], &headers),
    };
    // Any failure to talk to ListenBrainz is worth retrying later; the listens stay queued.
    let reply = reply.map_err(|error| {
        eprintln!("Could not reach ListenBrainz: {error}");
        Error::Unavailable
    })?;
    Ok((reply.status, reply.body))
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

/// A track from the user's ListenBrainz statistics.
#[derive(Debug, PartialEq)]
pub struct Recording {
    pub artist: String,
    pub title: String,
}

/// Reads a stats reply; `None` when ListenBrainz has not computed stats for that range yet.
fn parse_recordings(status: u16, body: &str) -> Result<Option<Vec<Recording>>, Error> {
    match status {
        204 => return Ok(None),
        200 => {}
        401 => return Err(Error::Token),
        _ => return Err(Error::Unavailable),
    }
    let reply: serde_json::Value = serde_json::from_str(body).map_err(|_| Error::Unavailable)?;
    let recordings = reply["payload"]["recordings"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(Recording {
                        artist: item["artist_name"].as_str()?.to_owned(),
                        title: item["track_name"].as_str()?.to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Some(recordings))
}

fn encode_user(user: &str) -> String {
    user.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

/// The user's most played recordings: this month if computed, else this year, else all time.
/// `None` when ListenBrainz has not computed any statistics for the user yet.
pub fn top_recordings(token: &str, user: &str) -> Result<Option<Vec<Recording>>, Error> {
    let user = encode_user(user);
    for range in ["month", "year", "all_time"] {
        let url = format!("{API}/stats/user/{user}/recordings?range={range}&count=100");
        let (status, body) = request(token, &url, None)?;
        if let Some(recordings) = parse_recordings(status, &body)? {
            return Ok(Some(recordings));
        }
    }
    Ok(None)
}

/// How many listens ListenBrainz has recorded for the user.
pub fn listen_count(token: &str, user: &str) -> Result<u64, Error> {
    let url = format!("{API}/user/{}/listen-count", encode_user(user));
    let (status, body) = request(token, &url, None)?;
    check(status)?;
    let reply: serde_json::Value = serde_json::from_str(&body).map_err(|_| Error::Unavailable)?;
    reply["payload"]["count"].as_u64().ok_or(Error::Unavailable)
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
    fn requests_send_the_token_header_and_body() {
        let (address, server) = crate::http::tests::serve_once(
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok",
            Duration::ZERO,
        );
        let body = submit_body(Kind::Single, &[listen()]);
        let reply = request(
            " secret-token \n",
            &format!("{address}/submit"),
            Some(&body),
        );
        assert_eq!(reply, Ok((200, "ok".to_owned())));
        let received = server.join().unwrap().unwrap();
        assert_eq!(received.request_line, "POST /submit HTTP/1.1");
        let has = |expected: &str| {
            received
                .headers
                .iter()
                .any(|header| header.eq_ignore_ascii_case(expected))
        };
        assert!(has("authorization: Token secret-token"));
        assert!(has("content-type: application/json"));
        assert_eq!(received.body, body);
    }

    #[test]
    fn unreachable_servers_leave_listens_for_later() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let reply = request("token", &format!("http://127.0.0.1:{port}/"), None);
        assert_eq!(reply, Err(Error::Unavailable));
    }

    #[test]
    fn stats_replies_list_recordings_or_report_no_stats_yet() {
        let body = r#"{"payload":{"recordings":[
            {"artist_name":"TM Juke","track_name":"Just for a Day (Sunday)","listen_count":243},
            {"artist_name":"Pretty Lights","listen_count":227}
        ]}}"#;
        let recordings = parse_recordings(200, body).unwrap().unwrap();
        assert_eq!(
            recordings,
            [Recording {
                artist: "TM Juke".into(),
                title: "Just for a Day (Sunday)".into()
            }]
        );
        assert_eq!(parse_recordings(204, ""), Ok(None));
        assert_eq!(parse_recordings(401, ""), Err(Error::Token));
        assert_eq!(parse_recordings(500, ""), Err(Error::Unavailable));
    }

    #[test]
    fn statuses_separate_bad_tokens_from_outages() {
        assert_eq!(check(200), Ok(()));
        assert_eq!(check(401), Err(Error::Token));
        assert_eq!(check(503), Err(Error::Unavailable));
    }
}
