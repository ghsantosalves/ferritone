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
    #[test]
    fn m3u_import_skips_comments_blank_lines_and_unsupported_entries() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("Old.m3u");
        fs::write(
            &path,
            "#EXTM3U\n\n# A comment\n#EXTINF:215,Artist - Song\nsong.mp3\n   \nnotes.txt\n  spaced.ogg  \n",
        )
        .unwrap();
        let imported = read(&path).unwrap();
        assert_eq!(imported.name, "Old");
        assert_eq!(
            imported.tracks,
            [
                directory.path().join("song.mp3"),
                directory.path().join("spaced.ogg"),
            ]
        );
    }

    #[test]
    fn m3u8_import_reads_utf8_relative_absolute_and_file_uri_entries() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("Viagem.m3u8");
        fs::write(
            &path,
            "#EXTM3U\r\n\
             #EXTINF:200,Ação - Canção\r\n\
             Pasta/Canção.flac\r\n\
             \r\n\
             #EXTINF:-1,Absolute\r\n\
             /música/álbum/faixa.opus\r\n\
             ../Acima.wav\r\n\
             file:///m%C3%BAsica/uri%20track.m4a\r\n\
             #EXTINF:10,Stream\r\n\
             http://example.org/live.mp3\r\n",
        )
        .unwrap();
        assert_eq!(
            read(&path).unwrap().tracks,
            [
                directory.path().join("Pasta/Canção.flac"),
                PathBuf::from("/música/álbum/faixa.opus"),
                directory.path().join("../Acima.wav"),
                PathBuf::from("/música/uri track.m4a"),
            ]
        );
    }

    #[test]
    fn existing_files_are_listed_by_their_real_path() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("Disc")).unwrap();
        fs::write(directory.path().join("Disc/one.wav"), b"").unwrap();
        let path = directory.path().join("Real.m3u8");
        fs::write(&path, "./Disc/../Disc/one.wav\n").unwrap();
        assert_eq!(
            read(&path).unwrap().tracks,
            [fs::canonicalize(directory.path().join("Disc/one.wav")).unwrap()]
        );
    }

    #[test]
    fn pls_import_handles_crlf_spacing_and_unsupported_entries() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("Radio.PLS");
        fs::write(
            &path,
            "[playlist]\r\nNumberOfEntries=3\r\nFile3 = /abs/third.mp3\r\nFile1=first.ogg\r\nLength1=-1\r\nFile2=http://example.org/stream\r\nVersion=2\r\n",
        )
        .unwrap();
        assert!(is_playlist(&path));
        assert_eq!(
            read(&path).unwrap().tracks,
            [
                directory.path().join("first.ogg"),
                PathBuf::from("/abs/third.mp3"),
            ]
        );
    }

    #[test]
    fn m3u8_export_reads_back_as_the_same_playlist() {
        let directory = tempfile::tempdir().unwrap();
        let tracks = vec![
            directory.path().join("Música/01 Intro.flac"),
            directory.path().join("#hash.wav"),
            PathBuf::from("/outra pasta/Canção.mp3"),
            directory.path().join("Música/01 Intro.flac"),
            PathBuf::from("/trailing space .ogg"),
        ];
        let playlist = Playlist {
            name: "Ida e volta".to_owned(),
            tracks: tracks.clone(),
        };
        let destination = directory.path().join("Ida e volta.m3u8");
        playlist.export_m3u8(&destination).unwrap();
        let imported = read(&destination).unwrap();
        assert_eq!(imported.name, "Ida e volta");
        assert_eq!(imported.tracks, tracks);
    }
}
