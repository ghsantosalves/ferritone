use std::{fs, path::Path, process::Command};

use lofty::{config::ParseOptions, file::TaggedFileExt, probe::Probe, tag::ItemKey};

#[derive(Clone, Default)]
pub enum Lyrics {
    #[default]
    Missing,
    Plain(String),
    Timed(Vec<(u64, String)>),
}

impl Lyrics {
    pub fn lookup_online(info: &crate::metadata::TrackInfo) -> Option<Self> {
        let title = info.title.as_deref()?;
        let artist = info.artist.as_deref()?;
        let mut fields = vec![("track_name", title), ("artist_name", artist)];
        if let Some(album) = info.album.as_deref() {
            fields.push(("album_name", album));
        }
        let duration = info
            .duration_ms
            .map(|ms| ms.saturating_add(500) / 1000)
            .filter(|s| (1..=3600).contains(s))
            .map(|s| s.to_string());
        if let Some(seconds) = duration.as_deref() {
            fields.push(("duration", seconds));
        }
        let (status, body) = lrclib_request("get", &fields)?;
        let exact = match status {
            200 => serde_json::from_str::<LyricsRecord>(&body).ok(),
            404 => None,
            _ => return None,
        };
        let fallback = match_search_result(exact.as_slice(), info);
        if matches!(fallback, Some(Self::Timed(_))) {
            return fallback;
        }
        let results = lrclib_request("search", &fields[..2])
            .filter(|(status, _)| *status == 200)
            .and_then(|(_, body)| serde_json::from_str::<Vec<LyricsRecord>>(&body).ok());
        let alternate = results
            .as_deref()
            .and_then(|records| match_search_result(records, info));
        if matches!(alternate, Some(Self::Timed(_))) {
            alternate
        } else {
            fallback.or(alternate)
        }
    }

    pub fn read(track: &Path) -> Self {
        if let Ok(text) = fs::read_to_string(track.with_extension("lrc")) {
            let lyrics = Self::parse(&text);
            if !matches!(lyrics, Self::Missing) {
                return lyrics;
            }
        }
        let file = Probe::open(track).and_then(|probe| {
            probe
                .options(
                    ParseOptions::new()
                        .read_properties(false)
                        .read_cover_art(false),
                )
                .read()
        });
        file.ok()
            .and_then(|file| {
                file.tags()
                    .iter()
                    .find_map(|tag| tag.get_string(&ItemKey::Lyrics))
                    .map(Self::parse)
            })
            .unwrap_or_default()
    }

    fn parse(text: &str) -> Self {
        let text = text.trim_start_matches('\u{feff}').trim();
        let mut timed = Vec::new();
        let mut plain = Vec::new();
        let mut offset = 0;
        for line in text.lines() {
            let mut remaining = line.trim();
            if let Some(value) = remaining
                .strip_prefix("[offset:")
                .and_then(|s| s.strip_suffix(']'))
            {
                if let Ok(value) = value.trim().parse::<i64>() {
                    offset = value;
                }
                continue;
            }
            let mut times = Vec::new();
            while let Some((tag, rest)) =
                remaining.strip_prefix('[').and_then(|s| s.split_once(']'))
            {
                let Some(time) = parse_time(tag) else { break };
                times.push(time);
                remaining = rest;
            }
            if times.is_empty() {
                let metadata = remaining
                    .strip_prefix('[')
                    .and_then(|value| value.strip_suffix(']'))
                    .and_then(|value| value.split_once(':'))
                    .is_some_and(|(key, _)| {
                        matches!(
                            key,
                            "ar" | "al" | "ti" | "au" | "by" | "length" | "re" | "ve"
                        )
                    });
                if !metadata {
                    plain.push(remaining);
                }
            } else {
                timed.extend(
                    times
                        .into_iter()
                        .map(|time| (time, remaining.trim().to_owned())),
                );
            }
        }
        if timed.iter().any(|(_, line)| !line.is_empty()) {
            for (time, _) in &mut timed {
                *time = time.saturating_add_signed(offset.saturating_neg());
            }
            timed.sort_by_key(|(time, _)| *time);
            Self::Timed(timed)
        } else {
            let text = plain.join("\n");
            if text.trim().is_empty() {
                Self::Missing
            } else {
                Self::Plain(text)
            }
        }
    }

    pub fn excerpt(&self, position_ms: u64, advance_ms: i32) -> [Option<&str>; 5] {
        let Self::Timed(lines) = self else {
            return [None; 5];
        };
        let position = i128::from(position_ms) + i128::from(advance_ms);
        let next = lines.partition_point(|(time, _)| i128::from(*time) <= position);
        std::array::from_fn(|index| {
            next.checked_add_signed(index as isize - 3)
                .and_then(|line| lines.get(line))
                .map(|(_, text)| text.as_str())
        })
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct LyricsRecord {
    track_name: String,
    artist_name: String,
    album_name: Option<String>,
    duration: Option<f64>,
    synced_lyrics: Option<String>,
    plain_lyrics: Option<String>,
}

impl LyricsRecord {
    fn duration_difference(&self, info: &crate::metadata::TrackInfo) -> f64 {
        self.duration
            .zip(info.duration_ms)
            .map_or(f64::INFINITY, |(seconds, ms)| {
                (seconds - ms as f64 / 1000.0).abs()
            })
    }

    fn matches_recording(&self, info: &crate::metadata::TrackInfo) -> bool {
        let local_album = info.album.as_deref().map(fold_search_name);
        let online_album = self.album_name.as_deref().map(fold_search_name);
        let album_mismatch = local_album
            .zip(online_album)
            .is_some_and(|(local, online)| {
                !local.is_empty() && !online.is_empty() && local != online
            });
        self.duration_difference(info) <= 2.0 && !album_mismatch
    }

    fn lyrics(&self, allow_sync: bool) -> Option<Lyrics> {
        if allow_sync {
            if let Some(Lyrics::Timed(lines)) = self.synced_lyrics.as_deref().map(Lyrics::parse) {
                return Some(Lyrics::Timed(lines));
            }
        }
        match self.plain_lyrics.as_deref().map(Lyrics::parse) {
            Some(Lyrics::Plain(text)) => Some(Lyrics::Plain(text)),
            _ => None,
        }
    }
}

fn match_search_result(
    records: &[LyricsRecord],
    info: &crate::metadata::TrackInfo,
) -> Option<Lyrics> {
    let title = fold_search_name(info.title.as_deref()?);
    let artist = fold_search_name(info.artist.as_deref()?);
    if title.is_empty() || artist.is_empty() {
        return None;
    }
    let album = info.album.as_deref().map(fold_search_name);
    records
        .iter()
        .filter(|record| {
            fold_search_name(&record.track_name) == title
                && fold_search_name(&record.artist_name) == artist
        })
        .filter_map(|record| {
            let duration_difference = record.duration_difference(info);
            let lyrics = record.lyrics(record.matches_recording(info))?;
            let synced = matches!(lyrics, Lyrics::Timed(_));
            let same_album =
                album.is_some() && record.album_name.as_deref().map(fold_search_name) == album;
            Some((lyrics, synced, same_album, duration_difference))
        })
        .min_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then_with(|| right.2.cmp(&left.2))
                .then_with(|| left.3.total_cmp(&right.3))
        })
        .map(|(lyrics, _, _, _)| lyrics)
}

fn fold_search_name(text: &str) -> String {
    crate::search::search_key(text)
        .chars()
        .filter(|character| character.is_alphanumeric())
        .collect()
}

fn lrclib_request(endpoint: &str, fields: &[(&str, &str)]) -> Option<(u16, String)> {
    let mut request = Command::new("curl");
    request.args([
        "-q",
        "--get",
        "--silent",
        "--max-time",
        "6",
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
    ]);
    for (name, value) in fields {
        request
            .arg("--data-urlencode")
            .arg(format!("{name}={value}"));
    }
    let output = request
        .arg(format!("https://lrclib.net/api/{endpoint}"))
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let output = String::from_utf8(output.stdout).ok()?;
    let (body, status) = output.rsplit_once('\n')?;
    Some((status.parse().ok()?, body.to_owned()))
}

fn parse_time(tag: &str) -> Option<u64> {
    let (minutes, seconds) = tag.split_once(':')?;
    let (seconds, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
    let minutes: u64 = minutes.parse().ok()?;
    let seconds: u64 = seconds.parse().ok()?;
    if seconds >= 60 || !fraction.bytes().all(|digit| digit.is_ascii_digit()) {
        return None;
    }
    let fraction = fraction
        .bytes()
        .take(3)
        .zip([100, 10, 1])
        .map(|(digit, scale)| u64::from(digit - b'0') * scale)
        .sum::<u64>();
    minutes
        .checked_mul(60_000)?
        .checked_add(seconds * 1000 + fraction)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_timed_lines_after_seeking_and_handles_breaks() {
        let lyrics = Lyrics::parse("[00:05.10][00:01.5]First\n[00:03.000]\n[offset:200]");
        assert_eq!(
            lyrics.excerpt(1299, 0),
            [None, None, None, Some("First"), Some("")]
        );
        assert_eq!(
            lyrics.excerpt(1300, 0),
            [None, None, Some("First"), Some(""), Some("First")]
        );
        assert_eq!(
            lyrics.excerpt(3890, 0),
            [None, Some("First"), Some(""), Some("First"), None]
        );
        assert_eq!(lyrics.excerpt(4900, 0)[2], Some("First"));
        assert_eq!(lyrics.excerpt(2700, 0)[2], Some("First"));
        assert_eq!(lyrics.excerpt(1100, 250)[2], Some("First"));
        assert_eq!(lyrics.excerpt(1450, -250)[2], None);
    }

    #[test]
    fn fallback_matches_portuguese_accents_and_checks_recording_length() {
        let info = crate::metadata::TrackInfo {
            title: Some("Canção da Manhã".into()),
            artist: Some("João de Sá".into()),
            album: None,
            duration_ms: Some(182_000),
        };
        let record = |artist: &str, duration| LyricsRecord {
            track_name: "Cancao Da Manha".into(),
            artist_name: artist.into(),
            album_name: None,
            duration: Some(duration),
            synced_lyrics: Some("[00:02.00]Verse".into()),
            plain_lyrics: Some("Verse".into()),
        };
        let results = vec![record("Another Artist", 182.0), record("Joao de Sa", 183.4)];
        let lyrics = match_search_result(&results, &info).expect("matching recording");
        assert_eq!(lyrics.excerpt(2_000, 0)[2], Some("Verse"));
        assert!(matches!(
            match_search_result(&[record("Joao de Sa", 240.0)], &info),
            Some(Lyrics::Plain(_))
        ));
        assert!(match_search_result(&[record("Another Artist", 182.0)], &info).is_none());
        let mut other_album = record("Joao de Sa", 182.0);
        other_album.album_name = Some("Live".into());
        let studio = crate::metadata::TrackInfo {
            album: Some("Studio".into()),
            ..info
        };
        assert!(matches!(
            match_search_result(&[other_album], &studio),
            Some(Lyrics::Plain(_))
        ));
    }
}
