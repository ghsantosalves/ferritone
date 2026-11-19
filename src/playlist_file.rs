use std::{
    fs, io,
    path::{Path, PathBuf},
};

use gtk::gio::{self, prelude::*};

use crate::library::{Playlist, supported, title};

pub fn is_playlist(path: &Path) -> bool {
    matches!(extension(path).as_deref(), Some("m3u" | "m3u8" | "pls"))
}

pub fn read(path: &Path) -> io::Result<Playlist> {
    let contents = fs::read(path)?;
    let contents = String::from_utf8_lossy(&contents);
    let contents = contents.trim_start_matches('\u{feff}');
    let entries: Vec<_> = if extension(path).as_deref() == Some("pls") {
        let mut entries: Vec<_> = contents
            .lines()
            .filter_map(|line| {
                let (key, value) = line.split_once('=')?;
                let key = key.trim().to_ascii_lowercase();
                let number = key.strip_prefix("file")?.parse::<usize>().ok()?;
                Some((number, value.trim()))
            })
            .collect();
        entries.sort_by_key(|(number, _)| *number);
        entries.into_iter().map(|(_, entry)| entry).collect()
    } else {
        contents
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect()
    };
    let directory = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let tracks = entries
        .into_iter()
        .filter_map(|entry| resolve(directory, entry))
        .collect();
    Ok(Playlist {
        name: title(path),
        tracks,
    })
}

fn extension(path: &Path) -> Option<String> {
    path.extension()?.to_str().map(str::to_ascii_lowercase)
}

fn resolve(directory: &Path, entry: &str) -> Option<PathBuf> {
    if entry.is_empty() || entry.contains('\0') {
        return None;
    }
    let track = if entry.starts_with("file:") {
        gio::File::for_uri(entry).path()?
    } else if entry.contains("://") {
        return None;
    } else {
        directory.join(entry)
    };
    supported(&track).then(|| fs::canonicalize(&track).unwrap_or(track))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m3u8_import_preserves_order_and_duplicate_local_references() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("Mix.m3u8");
        fs::write(
            &path,
            "\u{feff}#EXTM3U\r\n./Música.wav\r\nhttps://example.org/stream.mp3\r\n./Música.wav\r\n",
        )
        .unwrap();
        let imported = read(&path).unwrap();
        assert_eq!(imported.name, "Mix");
        assert_eq!(
            imported.tracks,
            vec![directory.path().join("./Música.wav"); 2]
        );
    }

    #[test]
    fn pls_import_follows_numbered_entries() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("List.pls");
        fs::write(
            &path,
            "[playlist]\nFile2=second.flac\nTitle2=Second\nfile1=first.wav\n",
        )
        .unwrap();
        assert_eq!(
            read(&path).unwrap().tracks,
            [
                directory.path().join("first.wav"),
                directory.path().join("second.flac"),
            ]
        );
    }
}
