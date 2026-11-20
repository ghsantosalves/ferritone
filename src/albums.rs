//! Groups library tracks into albums and artists from their tags.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use crate::metadata::TrackInfo;

/// Identifies an album by its artist and title, ignoring case.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct AlbumKey {
    pub artist: String,
    pub title: String,
}

#[derive(Debug)]
pub struct Album {
    pub key: AlbumKey,
    /// Display names, as written in the first track's tags.
    pub artist: String,
    pub title: String,
    /// In disc and track order, then by file name.
    pub tracks: Vec<PathBuf>,
}

#[derive(Debug)]
pub struct Artist {
    pub key: String,
    pub name: String,
    pub albums: usize,
    pub tracks: usize,
}

/// The artist an album is filed under: the album artist, else the track artist.
pub fn album_artist(info: Option<&TrackInfo>) -> &str {
    info.and_then(|info| info.album_artist.as_deref().or(info.artist.as_deref()))
        .unwrap_or("")
}

fn fold(value: &str) -> String {
    value.trim().to_lowercase()
}

pub fn album_key(info: Option<&TrackInfo>) -> AlbumKey {
    AlbumKey {
        artist: fold(album_artist(info)),
        title: fold(info.and_then(|info| info.album.as_deref()).unwrap_or("")),
    }
}

/// Groups tracks into albums, sorted by artist and then title with `sort_key`.
pub fn group_albums<K: Ord>(
    tracks: &[PathBuf],
    info: &HashMap<PathBuf, TrackInfo>,
    sort_key: impl Fn(&str) -> K,
) -> Vec<Album> {
    let mut albums: Vec<Album> = Vec::new();
    let mut index: HashMap<AlbumKey, usize> = HashMap::new();
    for track in tracks {
        let metadata = info.get(track);
        let key = album_key(metadata);
        let position = *index.entry(key.clone()).or_insert_with(|| {
            albums.push(Album {
                key,
                artist: album_artist(metadata).to_owned(),
                title: metadata
                    .and_then(|info| info.album.clone())
                    .unwrap_or_default(),
                tracks: Vec::new(),
            });
            albums.len() - 1
        });
        albums[position].tracks.push(track.clone());
    }
    for album in &mut albums {
        album
            .tracks
            .sort_by(|a, b| track_order(a, info).cmp(&track_order(b, info)));
    }
    albums.sort_by(|a, b| {
        // Untitled albums and unknown artists go last.
        (
            a.key.artist.is_empty(),
            sort_key(&a.artist),
            a.key.title.is_empty(),
            sort_key(&a.title),
        )
            .cmp(&(
                b.key.artist.is_empty(),
                sort_key(&b.artist),
                b.key.title.is_empty(),
                sort_key(&b.title),
            ))
    });
    albums
}

fn track_order(track: &Path, info: &HashMap<PathBuf, TrackInfo>) -> (u32, u32, PathBuf) {
    let metadata = info.get(track);
    (
        metadata.and_then(|info| info.disc_number).unwrap_or(1),
        metadata
            .and_then(|info| info.track_number)
            .unwrap_or(u32::MAX),
        track.to_path_buf(),
    )
}

/// Summarizes albums by artist, in the albums' order.
pub fn group_artists(albums: &[Album]) -> Vec<Artist> {
    let mut artists: Vec<Artist> = Vec::new();
    for album in albums {
        match artists.last_mut() {
            Some(artist) if artist.key == album.key.artist => {
                artist.albums += 1;
                artist.tracks += album.tracks.len();
            }
            _ => artists.push(Artist {
                key: album.key.artist.clone(),
                name: album.artist.clone(),
                albums: 1,
                tracks: album.tracks.len(),
            }),
        }
    }
    artists
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(artist: &str, album: &str, album_artist: Option<&str>, track: u32) -> TrackInfo {
        TrackInfo {
            artist: Some(artist.into()),
            album: Some(album.into()),
            album_artist: album_artist.map(Into::into),
            track_number: Some(track),
            ..TrackInfo::default()
        }
    }

    #[test]
    fn tracks_group_by_album_artist_and_play_in_track_order() {
        let tracks: Vec<PathBuf> = ["c", "a", "b", "d", "e", "f"].map(PathBuf::from).into();
        let info = HashMap::from([
            (tracks[0].clone(), info("Zed", "Second", None, 2)),
            (tracks[1].clone(), info("zed", "second", None, 1)),
            (tracks[2].clone(), info("Amy", "First", None, 1)),
            // A compilation: different track artists, one album artist.
            (tracks[3].clone(), info("Bo", "Mix", Some("Various"), 1)),
            (tracks[4].clone(), info("Cy", "Mix", Some("Various"), 2)),
        ]);
        let albums = group_albums(&tracks, &info, str::to_lowercase);
        let titles: Vec<_> = albums.iter().map(|album| album.title.as_str()).collect();
        // "f" has no tags, so it lands in an untitled album by an unknown artist, last.
        assert_eq!(titles, ["First", "Mix", "Second", ""]);
        assert_eq!(albums[1].tracks, [tracks[3].clone(), tracks[4].clone()]);
        assert_eq!(albums[2].tracks, [tracks[1].clone(), tracks[0].clone()]);
        assert_eq!(albums[2].artist, "Zed");

        let artists = group_artists(&albums);
        let names: Vec<_> = artists
            .iter()
            .map(|artist| (artist.name.as_str(), artist.albums, artist.tracks))
            .collect();
        assert_eq!(
            names,
            [("Amy", 1, 1), ("Various", 1, 2), ("Zed", 1, 2), ("", 1, 1)]
        );
    }
}
