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
    /// Looks the track up on LRCLIB, trying looser queries until synchronized lyrics turn up.
    /// Missing tags are guessed from the file and folder names.
    pub fn lookup_online(tags: &crate::metadata::TrackInfo, track: &Path) -> Option<Self> {
        let info = fill_from_path(tags, track);
        let title = info.title.as_deref()?;
        let artist = info.artist.as_deref()?;
        let duration = info
            .duration_ms
            .map(|ms| ms.saturating_add(500) / 1000)
            .filter(|s| (1..=3600).contains(s))
            .map(|s| s.to_string());
        let core = core_title(title);
        let main_artist = primary_artist(artist);
        let mut records: Vec<LyricsRecord> = Vec::new();
        let mut best = None;
        // Stops as soon as synchronized lyrics match; each query widens the search.
        let mut attempt = |endpoint: &str, fields: Vec<(&str, &str)>| -> bool {
            let Some((200, body)) = lrclib_request(endpoint, &fields) else {
                return false;
            };
            if endpoint == "get" {
                records.extend(serde_json::from_str::<LyricsRecord>(&body).ok());
            } else {
                records.extend(
                    serde_json::from_str::<Vec<LyricsRecord>>(&body)
                        .unwrap_or_default()
                        .into_iter()
                        .take(40),
                );
            }
            best = match_search_result(&records, &info);
            matches!(best, Some(Self::Timed(_)))
        };
        let mut exact = vec![("track_name", title), ("artist_name", artist)];
        if let Some(album) = info.album.as_deref() {
            exact.push(("album_name", album));
        }
        if let Some(seconds) = duration.as_deref() {
            exact.push(("duration", seconds));
        }
        if attempt("get", exact) {
            return best;
        }
        let mut loose = vec![("track_name", core.as_str()), ("artist_name", main_artist)];
        if let Some(seconds) = duration.as_deref() {
            loose.push(("duration", seconds));
        }
        if (core != title || main_artist != artist) && attempt("get", loose) {
            return best;
        }
        if attempt(
            "search",
            vec![("track_name", core.as_str()), ("artist_name", main_artist)],
        ) {
            return best;
        }
        // LRCLIB may credit the artist differently ("&" for "e"); search by title alone and
        // let the local artist check decide.
        if attempt("search", vec![("track_name", core.as_str())]) {
            return best;
        }
        let words = format!("{main_artist} {core}");
        attempt("search", vec![("q", words.as_str())]);
        best
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

    /// Synchronized lyrics only line up with a recording of the same length. The album is not
    /// required to match: compilations and deluxe editions reuse the same recording.
    fn matches_recording(&self, info: &crate::metadata::TrackInfo) -> bool {
        self.duration_difference(info) <= 2.0
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
    let title = info.title.as_deref()?;
    let artist = info.artist.as_deref()?;
    if fold_search_name(title).is_empty() || fold_search_name(artist).is_empty() {
        return None;
    }
    let album = info.album.as_deref().map(fold_search_name);
    records
        .iter()
        .filter(|record| {
            titles_match(&record.track_name, title) && artists_match(&record.artist_name, artist)
        })
        .filter_map(|record| {
            let duration_difference = record.duration_difference(info);
            let lyrics = record.lyrics(record.matches_recording(info))?;
            let synced = matches!(lyrics, Lyrics::Timed(_));
            let exact_title = fold_search_name(&record.track_name) == fold_search_name(title);
            let same_album =
                album.is_some() && record.album_name.as_deref().map(fold_search_name) == album;
            Some((
                lyrics,
                (synced, exact_title, same_album),
                duration_difference,
            ))
        })
        // Synchronized first, then the exact title, the same album, and the closest length.
        .min_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then_with(|| left.2.total_cmp(&right.2))
        })
        .map(|(lyrics, _, _)| lyrics)
}

/// Words in brackets or after a dash that name a version rather than the song.
const VERSION_WORDS: [&str; 22] = [
    "remaster",
    "remastered",
    "live",
    "feat",
    "ft",
    "featuring",
    "with",
    "version",
    "edit",
    "mix",
    "remix",
    "mono",
    "stereo",
    "deluxe",
    "bonus",
    "explicit",
    "clean",
    "single",
    "radio",
    "acoustic",
    "demo",
    "bootleg",
];

fn names_version(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower
        .split(|character: char| !character.is_alphanumeric())
        .any(|word| {
            VERSION_WORDS.contains(&word) || (word.len() == 4 && word.starts_with(['1', '2']))
        })
}

/// The song title without version notes such as "(Remastered 2011)", "[Live]",
/// "(feat. Someone)", or " - 2009 Remaster".
pub fn core_title(title: &str) -> String {
    let mut text = String::new();
    let mut depth = 0usize;
    let mut group = String::new();
    for character in title.chars() {
        match character {
            '(' | '[' => {
                depth += 1;
                if depth == 1 {
                    group.clear();
                    continue;
                }
            }
            ')' | ']' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if !names_version(&group) {
                        text.push('(');
                        text.push_str(&group);
                        text.push(')');
                    }
                    continue;
                }
            }
            _ => {}
        }
        if depth > 0 {
            group.push(character);
        } else {
            text.push(character);
        }
    }
    let mut text = text.trim().to_owned();
    for separator in [" - ", " – ", " — "] {
        if let Some((head, tail)) = text.rsplit_once(separator) {
            if names_version(tail) && !head.trim().is_empty() {
                text = head.trim().to_owned();
            }
        }
    }
    for marker in [" feat. ", " feat ", " ft. ", " ft "] {
        if let Some(index) = text.to_lowercase().find(marker) {
            text.truncate(index);
        }
    }
    let text = text.trim().to_owned();
    if text.is_empty() {
        title.trim().to_owned()
    } else {
        text
    }
}

/// Each credited artist, from "A feat. B", "A & B", "A, B", or "A x B".
fn artist_names(artist: &str) -> Vec<String> {
    let mut names = artist.to_lowercase();
    for separator in [
        " feat. ",
        " feat ",
        " ft. ",
        " ft ",
        " featuring ",
        " & ",
        " and ",
        " e ",
        " y ",
        " x ",
        " with ",
        ", ",
        "; ",
        " / ",
    ] {
        names = names.replace(separator, "\u{1}");
    }
    names
        .split('\u{1}')
        .map(fold_search_name)
        .filter(|name| !name.is_empty())
        .collect()
}

/// The first credited artist, for queries.
pub fn primary_artist(artist: &str) -> &str {
    let lower = artist.to_lowercase();
    let cut = [
        " feat. ",
        " feat ",
        " ft. ",
        " ft ",
        " featuring ",
        " & ",
        ", ",
        "; ",
        " / ",
        " x ",
        " with ",
        " e ",
        " and ",
        " y ",
    ]
    .iter()
    .filter_map(|separator| lower.find(separator))
    .min()
    .unwrap_or(artist.len());
    // Lowercasing keeps byte offsets for the separators, which are ASCII.
    artist.get(..cut).unwrap_or(artist).trim()
}

fn titles_match(online: &str, local: &str) -> bool {
    let (online_exact, local_exact) = (fold_search_name(online), fold_search_name(local));
    online_exact == local_exact
        || (!local_exact.is_empty()
            && fold_search_name(&core_title(online)) == fold_search_name(&core_title(local)))
}

fn artists_match(online: &str, local: &str) -> bool {
    if fold_search_name(online) == fold_search_name(local) {
        return true;
    }
    let local = artist_names(local);
    artist_names(online).iter().any(|name| local.contains(name))
}

/// Fills a missing title or artist from names like "01 - Artist - Title.mp3", or from the
/// "Artist/Album/01 Title.mp3" folder layout.
pub fn fill_from_path(
    info: &crate::metadata::TrackInfo,
    track: &Path,
) -> crate::metadata::TrackInfo {
    let mut info = info.clone();
    if info.title.is_some() && info.artist.is_some() {
        return info;
    }
    let stem = track
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    // Drop a leading track number such as "01 ", "01. ", "1-03 - ".
    let name = stem
        .trim_start_matches(|character: char| {
            character.is_ascii_digit() || "-._ ".contains(character)
        })
        .trim();
    let name = if name.is_empty() { stem.trim() } else { name };
    let (guessed_artist, guessed_title) = match name.split_once(" - ") {
        Some((artist, title)) => (Some(artist.trim().to_owned()), title.trim().to_owned()),
        None => (None, name.to_owned()),
    };
    if info.title.is_none() && !guessed_title.is_empty() {
        info.title = Some(guessed_title);
    }
    if info.artist.is_none() {
        info.artist = guessed_artist.or_else(|| {
            let album_folder = track.parent()?;
            let artist_folder = album_folder.parent()?;
            let name = artist_folder
                .file_name()?
                .to_string_lossy()
                .trim()
                .to_owned();
            (!name.is_empty()).then_some(name)
        });
    }
    info
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
            ..Default::default()
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
        // The same recording on another album, such as a compilation, still syncs.
        let mut other_album = record("Joao de Sa", 182.0);
        other_album.album_name = Some("Greatest Hits".into());
        let studio = crate::metadata::TrackInfo {
            album: Some("Studio".into()),
            ..info.clone()
        };
        assert!(matches!(
            match_search_result(&[other_album], &studio),
            Some(Lyrics::Timed(_))
        ));
    }

    #[test]
    fn version_notes_and_extra_artists_still_find_the_song() {
        let info = crate::metadata::TrackInfo {
            title: Some("Show Me How to Live (2011 Remaster)".into()),
            artist: Some("Audioslave feat. Someone".into()),
            album: Some("Audioslave (Deluxe)".into()),
            duration_ms: Some(277_000),
            ..Default::default()
        };
        let record = LyricsRecord {
            track_name: "Show Me How To Live".into(),
            artist_name: "Audioslave".into(),
            album_name: Some("Audioslave".into()),
            duration: Some(277.4),
            synced_lyrics: Some("[00:10.00]Nail in my hand".into()),
            plain_lyrics: Some("Nail in my hand".into()),
        };
        assert!(matches!(
            match_search_result(&[record], &info),
            Some(Lyrics::Timed(_))
        ));
    }

    #[test]
    fn core_titles_drop_version_notes_but_keep_real_parentheses() {
        for (title, core) in [
            ("Song (Remastered 2011)", "Song"),
            ("Song - 2009 Remaster", "Song"),
            ("Song [Live]", "Song"),
            ("Song (feat. Someone)", "Song"),
            ("Song feat. Someone", "Song"),
            ("Song - Radio Edit", "Song"),
            (
                "(I Can't Get No) Satisfaction",
                "(I Can't Get No) Satisfaction",
            ),
            ("Love Song - Part One", "Love Song - Part One"),
            ("(Live)", "(Live)"),
        ] {
            assert_eq!(core_title(title), core, "{title}");
        }
        assert_eq!(primary_artist("Daft Punk feat. Pharrell"), "Daft Punk");
        assert_eq!(primary_artist("Simon & Garfunkel"), "Simon");
        assert_eq!(
            primary_artist("Chico Science e Nação Zumbi"),
            "Chico Science"
        );
        assert!(artists_match(
            "Chico Science & Nação Zumbi",
            "Chico Science e Nação Zumbi"
        ));
        assert!(artists_match("Simon & Garfunkel", "Simon & Garfunkel"));
        assert!(artists_match("Daft Punk", "Daft Punk, Pharrell Williams"));
        assert!(!artists_match("Daft Punk", "Air"));
    }

    #[test]
    fn missing_tags_are_guessed_from_file_and_folder_names() {
        let empty = crate::metadata::TrackInfo::default();
        let named = fill_from_path(
            &empty,
            Path::new("/music/Mix/03 - Audioslave - Like a Stone.flac"),
        );
        assert_eq!(named.artist.as_deref(), Some("Audioslave"));
        assert_eq!(named.title.as_deref(), Some("Like a Stone"));
        let foldered = fill_from_path(
            &empty,
            Path::new("/music/Audioslave/Audioslave/04 Cochise.mp3"),
        );
        assert_eq!(foldered.artist.as_deref(), Some("Audioslave"));
        assert_eq!(foldered.title.as_deref(), Some("Cochise"));
        let tagged = crate::metadata::TrackInfo {
            title: Some("Tagged".into()),
            artist: Some("Artist".into()),
            ..Default::default()
        };
        assert_eq!(
            fill_from_path(&tagged, Path::new("/x/01 Other.mp3"))
                .title
                .as_deref(),
            Some("Tagged")
        );
    }
}
