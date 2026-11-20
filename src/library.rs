use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Library {
    pub tracks: Vec<PathBuf>,
    pub library_roots: BTreeSet<PathBuf>,
    pub hidden_library_tracks: BTreeSet<PathBuf>,
    pub hidden_library_folders: BTreeSet<PathBuf>,
    pub lyric_offsets_ms: BTreeMap<PathBuf, i32>,
    pub playlists: Vec<Playlist>,
    pub queue: Vec<PathBuf>,
    pub queue_playlist: Option<usize>,
    pub current: usize,
    pub position_ms: u64,
    pub language: Language,
    pub theme: Theme,
    pub accent: Accent,
    pub repeat: Repeat,
    pub shuffle: bool,
    pub shuffle_order: Vec<usize>,
    pub view: ViewOptions,
    pub sort: TrackSort,
    pub descending: bool,
    pub equalizer: crate::equalizer::Settings,
    pub replay_gain: crate::replay_gain::Settings,
    pub mini_visual: MiniVisual,
    pub mini_display: Option<MiniDisplay>,
    pub remote_enabled: bool,
    /// The secret in the phone remote's link; kept so a bookmarked link keeps working.
    pub remote_token: String,
    /// How many times each track has been played past the halfway point.
    pub play_counts: BTreeMap<PathBuf, u32>,
    /// When each track was last played, in seconds since the Unix epoch.
    pub last_played: BTreeMap<PathBuf, u64>,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(default)]
pub struct ViewOptions {
    pub sidebar: bool,
    pub library_actions: bool,
    pub playlist_actions: bool,
    pub playlist_drop_zone: bool,
    pub search_button: bool,
    pub titlebar_heading: bool,
    pub titlebar_icon: bool,
    pub context_menu_icons: bool,
    pub context_menu_keycaps: bool,
    pub columns: [bool; 3],
    pub column_widths: [i32; 3],
    pub column_order: [usize; 4],
    pub transport_height: i32,
    pub lyrics_height: i32,
    pub transport_title_width: i32,
    pub cover_thumbnail: bool,
    pub track_info: bool,
    pub seek_bar: bool,
    pub elapsed: bool,
    pub playback_modes: bool,
    pub volume: bool,
    pub volume_percent: bool,
    pub lyrics: bool,
    pub online_lyrics: bool,
}

impl Default for ViewOptions {
    fn default() -> Self {
        Self {
            sidebar: true,
            library_actions: true,
            playlist_actions: true,
            playlist_drop_zone: true,
            search_button: false,
            titlebar_heading: false,
            titlebar_icon: true,
            context_menu_icons: true,
            context_menu_keycaps: true,
            columns: [true; 3],
            column_widths: [125, 125, 67],
            column_order: [0, 1, 2, 3],
            transport_height: 0,
            lyrics_height: 0,
            transport_title_width: 0,
            cover_thumbnail: true,
            track_info: true,
            seek_bar: true,
            elapsed: true,
            playback_modes: true,
            volume: true,
            volume_percent: false,
            lyrics: false,
            online_lyrics: false,
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
pub enum TrackSort {
    #[default]
    Title,
    Artist,
    Album,
    Duration,
}

#[derive(Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
pub enum MiniVisual {
    #[default]
    Cover,
    Waveform,
    Bars,
    Mirror,
    Lyrics,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
pub struct MiniDisplay {
    pub cover: bool,
    pub waveform: bool,
    pub lyrics: bool,
    pub waveform_style: MiniVisual,
}

impl From<MiniVisual> for MiniDisplay {
    fn from(visual: MiniVisual) -> Self {
        let waveform = matches!(
            visual,
            MiniVisual::Waveform | MiniVisual::Bars | MiniVisual::Mirror
        );
        Self {
            cover: visual == MiniVisual::Cover,
            waveform,
            lyrics: visual == MiniVisual::Lyrics,
            waveform_style: if waveform {
                visual
            } else {
                MiniVisual::Waveform
            },
        }
    }
}

impl ViewOptions {
    pub fn ordered_columns(&self) -> [usize; 4] {
        let mut sorted = self.column_order;
        sorted.sort();
        if sorted == [0, 1, 2, 3] {
            self.column_order
        } else {
            [0, 1, 2, 3]
        }
    }

    pub fn move_column(&mut self, from: usize, to: usize) -> bool {
        let order = self.ordered_columns();
        let Some(from) = order.iter().position(|column| *column == from) else {
            return false;
        };
        let Some(to) = order.iter().position(|column| *column == to) else {
            return false;
        };
        if from == to {
            return false;
        }
        let mut order = order.to_vec();
        let column = order.remove(from);
        order.insert(to, column);
        self.column_order.copy_from_slice(&order);
        true
    }
}

#[derive(Deserialize, Serialize)]
pub struct Playlist {
    pub name: String,
    pub tracks: Vec<PathBuf>,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
pub enum Language {
    #[default]
    System,
    English,
    Portuguese,
    Spanish,
    French,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
pub enum Theme {
    #[default]
    Ferritone,
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
pub enum Accent {
    #[default]
    Copper,
    System,
    Green,
    Blue,
    Violet,
}

impl Accent {
    pub fn color(self, desktop_accent: Option<&str>) -> &'static str {
        match self {
            Self::System => match desktop_accent {
                Some("blue") => "#3584e4",
                Some("teal") => "#2190a4",
                Some("green") => "#3a944a",
                Some("yellow") => "#c88800",
                Some("orange") => "#ed5b00",
                Some("red") => "#e62d42",
                Some("pink") => "#d56199",
                Some("purple") => "#9141ac",
                Some("slate") => "#6f8396",
                _ => "@theme_selected_bg_color",
            },
            Self::Copper => "#ee853f",
            Self::Green => "#78bc4a",
            Self::Blue => "#669de7",
            Self::Violet => "#ac89e4",
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
pub enum Repeat {
    #[default]
    Off,
    Queue,
    Track,
}

impl Playlist {
    pub fn export_m3u8(&self, destination: &Path) -> io::Result<()> {
        use std::io::Write;

        let parent = destination.parent().ok_or(io::ErrorKind::InvalidInput)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        writeln!(file, "#EXTM3U")?;
        for track in &self.tracks {
            let relative = track.strip_prefix(parent).ok();
            match relative.and_then(Path::to_str).or_else(|| track.to_str()) {
                Some(text) if !text.contains(['\n', '\r']) && text.trim() == text => {
                    if relative.is_some() {
                        write!(file, "./")?;
                    }
                    writeln!(file, "{text}")?;
                }
                _ => {
                    use gtk::gio::prelude::FileExt;
                    writeln!(file, "{}", gtk::gio::File::for_path(track).uri())?;
                }
            }
        }
        file.as_file().sync_all()?;
        file.persist(destination).map_err(|error| error.error)?;
        Ok(())
    }
}

impl Language {
    pub fn resolve(self) -> Self {
        match self {
            Self::System => gtk::glib::language_names()
                .iter()
                .find_map(|locale| match locale.split(['_', '-', '.', '@']).next()? {
                    "en" => Some(Self::English),
                    "pt" => Some(Self::Portuguese),
                    "es" => Some(Self::Spanish),
                    "fr" => Some(Self::French),
                    _ => None,
                })
                .unwrap_or(Self::English),
            explicit => explicit,
        }
    }

    pub fn text<'a>(self, english: &'a str, portuguese: &'a str) -> &'a str {
        match self {
            Self::Portuguese => portuguese,
            Self::Spanish => crate::i18n::lookup(english, 0).unwrap_or(english),
            Self::French => crate::i18n::lookup(english, 1).unwrap_or(english),
            _ => english,
        }
    }
}

impl Library {
    pub fn library_track_visible(&self, track: &Path) -> bool {
        !self.hidden_library_tracks.contains(track)
            && !track
                .ancestors()
                .any(|parent| self.hidden_library_folders.contains(parent))
    }

    pub fn remove_playlist(&mut self, index: usize) {
        self.playlists.remove(index);
        self.queue_playlist = self.queue_playlist.and_then(|source| {
            if source == index {
                None
            } else if source > index {
                Some(source - 1)
            } else {
                Some(source)
            }
        });
    }

    pub fn reset_shuffle_order(&mut self) {
        self.shuffle_order.clear();
        if !self.shuffle || self.current >= self.queue.len() {
            return;
        }

        let mut remaining: Vec<_> = (0..self.queue.len())
            .filter(|index| *index != self.current)
            .collect();
        for index in (1..remaining.len()).rev() {
            let other = gtk::glib::random_int_range(0, index as i32 + 1) as usize;
            remaining.swap(index, other);
        }
        self.shuffle_order.push(self.current);
        self.shuffle_order.extend(remaining);
    }

    pub fn ensure_shuffle_order(&mut self) {
        if !self.shuffle {
            self.shuffle_order.clear();
            return;
        }
        let indexes: BTreeSet<_> = self.shuffle_order.iter().copied().collect();
        if self.shuffle_order.len() != self.queue.len()
            || !indexes.iter().copied().eq(0..self.queue.len())
        {
            self.reset_shuffle_order();
        }
    }

    pub fn move_queue_track(&mut self, from: usize, to: usize) -> bool {
        if from == to || from >= self.queue.len() || to >= self.queue.len() {
            return false;
        }
        if self.shuffle {
            self.ensure_shuffle_order();
            if self.shuffle_order.len() != self.queue.len() {
                return false;
            }
            let track = self.shuffle_order.remove(from);
            self.shuffle_order.insert(to, track);
        } else {
            let track = self.queue.remove(from);
            self.queue.insert(to, track);
            self.current = match self.current {
                index if index == from => to,
                index if from < index && index <= to => index - 1,
                index if to <= index && index < from => index + 1,
                index => index,
            };
        }
        true
    }

    pub fn insert_queue_tracks(&mut self, after: Option<usize>, tracks: Vec<PathBuf>) {
        if tracks.is_empty() {
            return;
        }
        self.ensure_shuffle_order();
        let after = after.filter(|index| *index < self.queue.len());
        let at = after.map_or(self.queue.len(), |index| index + 1);
        let count = tracks.len();
        let had_current = self.current < self.queue.len();
        let playback_position = self
            .shuffle_order
            .iter()
            .position(|&index| Some(index) == after)
            .map_or(self.shuffle_order.len(), |position| position + 1);
        self.queue.splice(at..at, tracks);
        if had_current && self.current >= at {
            self.current += count;
        }
        if self.shuffle {
            for index in &mut self.shuffle_order {
                if *index >= at {
                    *index += count;
                }
            }
            self.shuffle_order
                .splice(playback_position..playback_position, at..at + count);
        }
    }

    /// Returns whether the current queue entry was removed.
    pub fn remove_queue_tracks(&mut self, indexes: &BTreeSet<usize>) -> bool {
        self.ensure_shuffle_order();
        let was_current = self.current;
        let mut new_indexes = vec![None; self.queue.len()];
        let mut old_index = 0;
        let mut new_index = 0;
        self.queue.retain(|_| {
            let keep = !indexes.contains(&old_index);
            if keep {
                new_indexes[old_index] = Some(new_index);
                new_index += 1;
            }
            old_index += 1;
            keep
        });
        let current_removed = indexes.contains(&was_current);
        let replacement = if current_removed && self.shuffle {
            self.shuffle_order
                .iter()
                .skip_while(|&&index| index != was_current)
                .find_map(|&index| new_indexes[index])
                .or_else(|| {
                    self.shuffle_order
                        .iter()
                        .rev()
                        .find_map(|&index| new_indexes[index])
                })
        } else {
            new_indexes
                .iter()
                .skip(was_current)
                .find_map(|&index| index)
        };
        self.current = replacement.unwrap_or_else(|| self.queue.len().saturating_sub(1));
        if current_removed {
            self.position_ms = 0;
        }
        if self.queue.is_empty() {
            self.queue_playlist = None;
        }
        self.shuffle_order = self
            .shuffle_order
            .iter()
            .filter_map(|&index| new_indexes[index])
            .collect();
        current_removed
    }

    pub fn next_index(&self, offset: isize, automatic: bool) -> Option<usize> {
        if self.current >= self.queue.len() {
            return None;
        }
        if automatic && self.repeat == Repeat::Track {
            return Some(self.current);
        }
        let position = if self.shuffle {
            self.shuffle_order
                .iter()
                .position(|index| *index == self.current)?
        } else {
            self.current
        };
        let target = position
            .checked_add_signed(offset)
            .filter(|index| *index < self.queue.len())
            .or_else(|| {
                (self.repeat == Repeat::Queue)
                    .then(|| if offset < 0 { self.queue.len() - 1 } else { 0 })
            })?;
        if self.shuffle {
            self.shuffle_order.get(target).copied()
        } else {
            Some(target)
        }
    }

    pub fn record_play(&mut self, track: &Path, now: u64) {
        *self.play_counts.entry(track.to_path_buf()).or_default() += 1;
        self.last_played.insert(track.to_path_buf(), now);
    }

    /// The most played tracks, ties broken by the most recent play.
    pub fn most_played(&self, limit: usize) -> Vec<PathBuf> {
        let mut tracks: Vec<_> = self.play_counts.iter().collect();
        tracks.sort_by(|(a, a_count), (b, b_count)| {
            b_count
                .cmp(a_count)
                .then_with(|| self.last_played.get(*b).cmp(&self.last_played.get(*a)))
        });
        tracks
            .into_iter()
            .take(limit)
            .map(|(track, _)| track.clone())
            .collect()
    }

    /// The tracks played most recently, newest first.
    pub fn recently_played(&self, limit: usize) -> Vec<PathBuf> {
        let mut tracks: Vec<_> = self.last_played.iter().collect();
        tracks.sort_by(|(_, a), (_, b)| b.cmp(a));
        tracks
            .into_iter()
            .take(limit)
            .map(|(track, _)| track.clone())
            .collect()
    }

    /// Brings the library in line with fresh scans of its folders. Only folders that were
    /// scanned are touched, so an unplugged drive does not empty the library.
    /// Returns how many tracks were added and removed.
    pub fn sync_roots(&mut self, scans: &[RootScan]) -> (usize, usize) {
        let before = self.tracks.len();
        let mut removed = 0;
        for scan in scans {
            let found: BTreeSet<&PathBuf> = scan.tracks.iter().collect();
            self.tracks.retain(|track| {
                let keep = !track.starts_with(&scan.root) || found.contains(track);
                removed += usize::from(!keep);
                keep
            });
        }
        let known: BTreeSet<PathBuf> = self.tracks.iter().cloned().collect();
        let new: BTreeSet<&PathBuf> = scans
            .iter()
            .flat_map(|scan| &scan.tracks)
            .filter(|track| !known.contains(*track))
            .collect();
        let added = new.len();
        self.tracks.extend(new.into_iter().cloned());
        self.tracks.sort();
        debug_assert_eq!(self.tracks.len(), before - removed + added);
        (added, removed)
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        let mut library: Self = match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(error) => return Err(error),
        };
        // A resume point newer than the library; ignored if unreadable or for another track.
        let resume = fs::read(resume_path(path))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<ResumePoint>(&bytes).ok());
        if let Some(resume) = resume {
            if library.queue.get(resume.current) == Some(&resume.track) {
                library.current = resume.current;
                library.position_ms = resume.position_ms;
            }
        }
        Ok(library)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let directory = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
        fs::create_dir_all(directory)?;
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        serde_json::to_writer_pretty(file.as_file_mut(), self)?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|error| error.error)?;
        // The library now holds the latest position, so the resume point is stale.
        match fs::remove_file(resume_path(path)) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    /// Saves only the playback position, which is much cheaper than `save` for a large library.
    pub fn save_position(&self, path: &Path) -> io::Result<()> {
        let Some(track) = self.queue.get(self.current) else {
            return Ok(());
        };
        let directory = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
        fs::create_dir_all(directory)?;
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        serde_json::to_writer(
            file.as_file_mut(),
            &ResumePoint {
                track: track.clone(),
                current: self.current,
                position_ms: self.position_ms,
            },
        )?;
        // No fsync: losing the latest few seconds of position after a crash is harmless.
        file.persist(resume_path(path))
            .map_err(|error| error.error)?;
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
struct ResumePoint {
    track: PathBuf,
    current: usize,
    position_ms: u64,
}

fn resume_path(library: &Path) -> PathBuf {
    library.with_file_name("position.json")
}

/// The newest files among `tracks` by modification time, newest first; unreadable files are skipped.
pub fn recently_added(tracks: &[PathBuf], limit: usize) -> Vec<PathBuf> {
    let mut dated: Vec<_> = tracks
        .iter()
        .filter_map(|track| Some((fs::metadata(track).ok()?.modified().ok()?, track)))
        .collect();
    dated.sort_by(|(a, _), (b, _)| b.cmp(a));
    dated
        .into_iter()
        .take(limit)
        .map(|(_, track)| track.clone())
        .collect()
}

pub fn data_path() -> PathBuf {
    gtk::glib::user_data_dir().join("rust-music-player/library.json")
}

pub fn collect_tracks(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut pending = paths;
    let mut visited = BTreeSet::new();
    let mut tracks = BTreeSet::new();
    while let Some(path) = pending.pop() {
        let Ok(path) = path.canonicalize() else {
            continue;
        };
        if !visited.insert(path.clone()) {
            continue;
        }
        if path.is_dir() {
            if let Ok(entries) = fs::read_dir(path) {
                pending.extend(entries.flatten().map(|entry| entry.path()));
            }
        } else if path.is_file() && supported(&path) {
            tracks.insert(path);
        }
    }
    tracks.into_iter().collect()
}

/// The audio files and folders found under one library folder.
pub struct RootScan {
    pub root: PathBuf,
    pub tracks: Vec<PathBuf>,
    pub folders: Vec<PathBuf>,
}

/// Scans a library folder, or returns `None` if it is not there right now.
pub fn scan_root(root: &Path) -> Option<RootScan> {
    let root = root.canonicalize().ok().filter(|root| root.is_dir())?;
    let mut pending = vec![root.clone()];
    let mut visited = BTreeSet::new();
    let (mut tracks, mut folders) = (Vec::new(), Vec::new());
    while let Some(folder) = pending.pop() {
        let Ok(folder) = folder.canonicalize() else {
            continue;
        };
        if !visited.insert(folder.clone()) {
            continue;
        }
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        folders.push(folder);
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.is_file() && supported(&path) {
                if let Ok(path) = path.canonicalize() {
                    tracks.push(path);
                }
            }
        }
    }
    tracks.sort();
    tracks.dedup();
    Some(RootScan {
        root,
        tracks,
        folders,
    })
}

pub fn supported(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "mp3" | "flac" | "ogg" | "oga" | "opus" | "wav" | "m4a" | "aac" | "aiff" | "alac"
            )
        })
}

pub fn title(path: &Path) -> String {
    path.file_stem()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_hides_selected_tracks_and_folders_by_path_component() {
        let mut state = Library::default();
        let folder = PathBuf::from("/music/live");
        let track = PathBuf::from("/music/studio/track.flac");
        state.hidden_library_folders.insert(folder.clone());
        state.hidden_library_tracks.insert(track.clone());
        assert!(!state.library_track_visible(Path::new("/music/live/disc/track.flac")));
        assert!(state.library_track_visible(Path::new("/music/live-sessions/track.flac")));
        assert!(!state.library_track_visible(&track));
        state.hidden_library_folders.remove(&folder);
        state.hidden_library_tracks.remove(&track);
        assert!(state.library_track_visible(Path::new("/music/live/disc/track.flac")));
        assert!(state.library_track_visible(&track));
    }

    #[test]
    fn deleting_a_playlist_rebases_its_source_without_altering_the_queue() {
        let tracks = vec![PathBuf::from("same.wav")];
        let mut state = Library {
            playlists: (0..3)
                .map(|_| Playlist {
                    name: "Duplicate".to_owned(),
                    tracks: tracks.clone(),
                })
                .collect(),
            queue: tracks.clone(),
            queue_playlist: Some(2),
            ..Library::default()
        };
        state.remove_playlist(1);
        assert_eq!(state.queue_playlist, Some(1));
        state.remove_playlist(1);
        assert_eq!(state.queue_playlist, None);
        assert_eq!(state.queue, tracks);
    }

    #[test]
    fn shuffle_traverses_each_track_and_keeps_the_original_order() {
        let mut library = Library {
            queue: ["first", "second", "third", "fourth"]
                .into_iter()
                .map(PathBuf::from)
                .collect(),
            current: 1,
            shuffle: true,
            ..Library::default()
        };
        let original = library.queue.clone();
        library.reset_shuffle_order();
        let mut played = BTreeSet::from([library.current]);
        while let Some(next) = library.next_index(1, true) {
            assert!(played.insert(next));
            assert_eq!(library.next_index(1, false), Some(next));
            library.current = next;
        }
        assert_eq!(played.len(), original.len());
        assert_eq!(library.queue, original);
        library.repeat = Repeat::Queue;
        assert_eq!(library.next_index(1, true), Some(1));
    }

    #[test]
    fn m3u8_keeps_track_order_and_uses_relative_paths_below_export_folder() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("Portable.m3u8");
        let external = PathBuf::from("/external/Música.wav");
        let playlist = Playlist {
            name: "Portable".to_owned(),
            tracks: vec![directory.path().join("Music/#Intro.wav"), external],
        };
        playlist.export_m3u8(&destination).unwrap();
        assert_eq!(
            fs::read_to_string(destination).unwrap(),
            "#EXTM3U\n./Music/#Intro.wav\n/external/Música.wav\n"
        );
    }

    #[test]
    fn moving_queue_rows_preserves_the_current_track_and_position() {
        let mut library = Library {
            queue: ["first", "second", "third"]
                .into_iter()
                .map(PathBuf::from)
                .collect(),
            current: 1,
            position_ms: 2500,
            ..Library::default()
        };
        assert!(library.move_queue_track(0, 2));
        assert_eq!(library.current, 0);
        assert_eq!(library.queue[library.current], PathBuf::from("second"));
        assert_eq!(library.position_ms, 2500);
        assert!(library.move_queue_track(0, 2));
        assert_eq!(library.current, 2);

        library.shuffle = true;
        library.shuffle_order = vec![2, 1, 0];
        let original = library.queue.clone();
        assert!(library.move_queue_track(2, 1));
        assert_eq!(library.queue, original);
        assert_eq!(library.current, 2);
        assert_eq!(library.next_index(1, true), Some(0));
    }

    #[test]
    fn editing_shuffled_queue_preserves_playback_position_and_duplicate_entries() {
        let mut state = Library {
            queue: ["same", "middle", "same"]
                .into_iter()
                .map(PathBuf::from)
                .collect(),
            current: 2,
            position_ms: 1234,
            shuffle: true,
            shuffle_order: vec![2, 0, 1],
            ..Library::default()
        };
        state.insert_queue_tracks(Some(2), vec![PathBuf::from("added")]);
        assert_eq!(state.shuffle_order, [2, 3, 0, 1]);
        assert!(!state.remove_queue_tracks(&BTreeSet::from([0])));
        assert_eq!((state.current, state.position_ms), (1, 1234));
        assert_eq!(state.shuffle_order, [1, 2, 0]);
        assert!(state.remove_queue_tracks(&BTreeSet::from([1])));
        assert_eq!(state.queue[state.current], PathBuf::from("added"));
        assert_eq!(state.position_ms, 0);
        assert_eq!(state.shuffle_order, [1, 0]);
    }

    #[test]
    fn resume_point_restores_the_position_until_the_library_is_saved() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("library.json");
        let mut library = Library {
            queue: ["first", "second"].into_iter().map(PathBuf::from).collect(),
            current: 1,
            position_ms: 1000,
            ..Library::default()
        };
        library.save(&path).unwrap();
        library.position_ms = 61_000;
        library.save_position(&path).unwrap();
        let loaded = Library::load(&path).unwrap();
        assert_eq!((loaded.current, loaded.position_ms), (1, 61_000));

        // A full save is newer than the resume point and replaces it.
        library.position_ms = 75_000;
        library.save(&path).unwrap();
        assert!(!resume_path(&path).exists());
        assert_eq!(Library::load(&path).unwrap().position_ms, 75_000);
    }

    #[test]
    fn resume_point_for_another_track_is_ignored() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("library.json");
        let mut library = Library {
            queue: ["first", "second"].into_iter().map(PathBuf::from).collect(),
            ..Library::default()
        };
        library.save(&path).unwrap();
        library.position_ms = 5000;
        library.save_position(&path).unwrap();
        library.queue = vec![PathBuf::from("other")];
        library.position_ms = 0;
        fs::write(&path, serde_json::to_vec(&library).unwrap()).unwrap();
        let loaded = Library::load(&path).unwrap();
        assert_eq!((loaded.current, loaded.position_ms), (0, 0));

        fs::write(resume_path(&path), b"not json").unwrap();
        assert!(Library::load(&path).is_ok());
    }

    #[test]
    fn play_history_ranks_most_and_recently_played_tracks() {
        let mut library = Library::default();
        let (a, b, c) = (Path::new("a"), Path::new("b"), Path::new("c"));
        library.record_play(a, 10);
        library.record_play(b, 20);
        library.record_play(a, 30);
        library.record_play(c, 40);
        assert_eq!(library.play_counts[a], 2);
        assert_eq!(library.most_played(2), [a, c]);
        assert_eq!(library.recently_played(10), [c, a, b]);
    }

    #[test]
    fn recently_added_orders_files_by_modification_time() {
        use std::time::{Duration, SystemTime};
        let directory = tempfile::tempdir().unwrap();
        let tracks: Vec<_> = ["old", "new", "middle"]
            .iter()
            .zip([100, 300, 200])
            .map(|(name, age)| {
                let path = directory.path().join(name);
                let file = fs::File::create(&path).unwrap();
                file.set_modified(SystemTime::now() - Duration::from_secs(age))
                    .unwrap();
                path
            })
            .collect();
        let missing = directory.path().join("missing");
        let mut all = tracks.clone();
        all.push(missing);
        let newest = recently_added(&all, 2);
        assert_eq!(newest, [tracks[0].clone(), tracks[2].clone()]);
    }

    #[test]
    fn syncing_a_folder_adds_new_files_and_drops_deleted_ones_only_there() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("Music");
        fs::create_dir_all(root.join("Album")).unwrap();
        for name in ["Album/one.flac", "Album/two.mp3", "notes.txt"] {
            fs::write(root.join(name), b"").unwrap();
        }
        let scan = scan_root(&root).unwrap();
        let root = scan.root.clone();
        assert_eq!(scan.tracks.len(), 2);
        assert_eq!(scan.folders.len(), 2);
        let elsewhere = PathBuf::from("/elsewhere/kept.ogg");
        let mut library = Library {
            tracks: vec![
                root.join("Album/one.flac"),
                root.join("gone.wav"),
                elsewhere.clone(),
            ],
            ..Library::default()
        };
        assert_eq!(library.sync_roots(&[scan]), (1, 1));
        assert!(library.tracks.contains(&root.join("Album/two.mp3")));
        assert!(!library.tracks.contains(&root.join("gone.wav")));
        assert!(library.tracks.contains(&elsewhere));
        // A folder that is not there (an unplugged drive) is not scanned and changes nothing.
        assert!(scan_root(&directory.path().join("Unplugged")).is_none());
        assert_eq!(library.sync_roots(&[]), (0, 0));
        assert_eq!(library.tracks.len(), 3);
    }

    #[test]
    fn repeat_track_only_affects_automatic_advancement() {
        let library = Library {
            queue: ["first", "second"].into_iter().map(PathBuf::from).collect(),
            repeat: Repeat::Track,
            ..Library::default()
        };
        assert_eq!(library.next_index(1, true), Some(0));
        assert_eq!(library.next_index(1, false), Some(1));
    }
}
