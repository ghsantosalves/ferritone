# Ferritone

Ferritone is a Linux desktop music player written in Rust. It organizes local audio into a searchable library, saved playlists, and a playback queue you can resume after restarting the app.

## Install

On Debian 13 (trixie) amd64, download the `.deb` from the [latest release](https://github.com/ghsantosalves/ferritone/releases/latest) and install it with apt, which also pulls in GTK and GStreamer:

```sh
sudo apt install ./ferritone_0.1.4-1.trixie1_amd64.deb
```

Open **Ferritone** from the application menu. To update, install the `.deb` from a newer release the same way. To uninstall, run `sudo apt remove ferritone`.

On other distributions, download the `.flatpak` file from the same release and install it with `flatpak install --user ./Ferritone-<version>-x86_64.flatpak`, or [build from source](#build).

## Get started

- **Home** shows the first six saved playlists and lets you open a folder, add a folder to the library, or resume the last queue. Search from Home to find a playlist anywhere in the collection. Opening a folder replaces the queue and starts playback. Adding a folder to the library indexes its tracks without changing playback. Both actions include subfolders.
- **Library** lets you add multiple folders and browse or search by title, artist, or album. You can mix local folders and cloud folders mounted or synced to the computer. **Manage library** lists the folder roots and lets you hide or show them independently. You can re-add a previously imported folder to register it as a source without duplicating tracks. Right-click selected tracks to hide them from the library. Hidden tracks stay in existing playlists and the queue, and are excluded when starting a new shuffle from the library. Use **View** in the playback menu to choose the visible columns and hide or show the library buttons.
- **Playlists** supports drag and drop for files and folders, local audio imports from M3U, M3U8, and PLS, and M3U8 export. Search saved playlists or search for tracks inside any open playlist. The active playlist and the currently playing row are marked. Right-click a saved playlist on Home or in Playlists to play it, add it to the queue, add tracks, rename it, export it, or delete it.
- **Queue** shows the playback order and supports track search. Drag a track by its handle or use the arrow buttons to reorder it. Reordering keeps the current track and playback position. Right-click a track to add a copy to the end of the queue or remove the selected entries. The repeat button cycles through off, repeat queue, and repeat track. Tracks follow each other without a gap, so live albums and continuous mixes play as recorded.
- **Track tables** sort when you click a column header. In the queue and playlists, the next click reverses the sort and the third restores the original order. Sorting these views keeps the playback sequence. Drag a header to move the column or drag its divider to resize it. Double-click a divider to fit the column to the tracks in the current list.
- The **playback menu** opens the mini player, the 10-band equalizer with presets, and View settings. Use the mini player's three-dot menu to show album art, live waveforms, and lyrics independently. Ferritone saves the combination and waveform style. The line uses rounded curves. Bars and the mirrored wave respond to the average energy of the audio and recede gradually between beats. The visualizations settle when playback is paused. The main progress slider hides when space is limited. The time display stays visible if enabled.
- **View** lets you search its options and controls the visibility of interface elements, including the sidebar, title bar icon, playlist buttons, and drop area. Playlist controls can be hidden in the overview and in individual playlists. When the overview drop area is hidden, you can still drop files onto the list to create or add to a playlist. Drag the top edge of the playback bar to resize its height or drag the divider next to the track information to resize its width.
- **Preferences** lets you change the theme, accent color, and language. The default theme uses charcoal and copper. Ferritone follows the system language by default and includes English, Brazilian Portuguese, Spanish, and French. Other locales fall back to English.

Ferritone currently plays local audio. Direct playback from a web URL is not implemented. File format support depends on the installed GStreamer plugins.

## Track actions and shortcuts

Use **Ctrl+click** to toggle individual tracks or **Shift+click** to select a range in the displayed order. Right-click a selected track to act on the selection. The context menu includes track details, queue actions, the containing folder, metadata refresh, and clipboard actions. Queue and playlist edits change the list of entries. The audio files stay in place.

Under **Preferences**, you can switch the small action icons and compact shortcut keycaps in context menus independently. When keycaps are off, shortcuts are shown as text.

| Shortcut | Action |
| --- | --- |
| Ctrl+O / Ctrl+K | Open music files / open a folder |
| Ctrl+Shift+O / Ctrl+Shift+K | Import a playlist / add a folder to the library |
| Ctrl+Shift+B | Manage library |
| Ctrl+F | Open search on the current page |
| Ctrl+P / Alt+← / Alt+→ | Play or pause / previous track / next track |
| Ctrl+E / Ctrl+M / Ctrl+Y | Equalizer / mini player / toggle lyrics |
| Alt+I / Alt+Q / F6 | Track details / add to queue or remove within the queue / refresh metadata |
| Ctrl+A / Ctrl+X / Ctrl+C | Select all visible tracks / cut / copy |
| Ctrl+V / Ctrl+Shift+V / Delete | Paste after selection / paste at end / hide selected library tracks or remove selected queue and playlist entries |
| Ctrl+Q | Close Ferritone |

In narrow windows, Ferritone can temporarily hide secondary columns. **View** reflects their current visibility, and widening the window restores the saved column choices.

Search matches names regardless of case or accents and shows the current result number and the total. Use the up and down arrows to highlight the previous or next match without starting playback. The arrows wrap at the ends. If you opened search with **Ctrl+F**, clicking outside the search controls closes the search and clears its filter.

## Optional lyrics

Enable **Lyrics** in **Preferences** or in the playback menu under **View**. You can also press **Ctrl+Y**. Ferritone looks for a UTF-8 `.lrc` file with the same name as the audio file in the same folder, then for lyrics embedded in the track's tags. The main player shows a single centered verse in a compact playback bar and adds the dimmed previous and next verses when space allows. Drag the top edge of the lyrics panel to resize it independently of the playback controls. A centered musical note marks the gaps between timed verses. The mini player's **Lyrics** panel reserves space for the timed excerpt so the window stays steady as verses change length. When timestamps are unavailable, the panel shows the full lyrics in a scrollable view.

Drag the timed lyrics up or down in the main player or mini player. Release when the highlighted verse matches what you hear to align that verse with the current playback position. Ferritone saves the adjustment for that audio file. **Reset timing** in the lyrics context menu restores the original timestamps.

Enable **Look up lyrics online** separately to search [LRCLIB](https://lrclib.net/docs) when local lyrics are unavailable. This option requires `curl` on the system. Ferritone sends the track title, artist, and available album and duration metadata. If the direct lookup has no synchronized lyrics, Ferritone checks alternate results for a compatible recording before falling back to plain text. The alternate search compares artist and title without accents or punctuation. Online lyrics are synchronized when durations are within two seconds and available album tags agree. A result from another edition can still be read as plain lyrics in the mini player. Audio and file paths stay on the device. Both lyric options are off by default. Online results are kept in memory for the current session.

## Desktop media controls

Ferritone exposes track details and playback controls over MPRIS. In GNOME, open the clock and notifications panel to access the current track, play or pause, and skip tracks. Desktop media keys work on environments that route them to MPRIS players.

Right-click Ferritone's icon in the dock or app grid for **Play/Pause**, **Previous Track**, and **Next Track**. If Ferritone is not running, choosing one of them opens it.

To show these controls when you hover over Ferritone's icon in the dock, install the optional GNOME Shell extension in `extras/gnome-shell-dock-controls` (GNOME 48 and 49; works with Dash to Dock and Ubuntu Dock):

```sh
uuid=ferritone-dock-controls@ghsantosalves.github.io
mkdir -p ~/.local/share/gnome-shell/extensions/$uuid
cp extras/gnome-shell-dock-controls/* ~/.local/share/gnome-shell/extensions/$uuid/
```

Log out and back in, then run `gnome-extensions enable $uuid`.

In **Preferences → Audio**, ReplayGain levels volume using embedded gain tags. **Track gain** suits mixed queues, and **Album gain** preserves the relative level of an album and falls back to track gain when needed. The optional peak protection caps boosting using the file's peak tag. Tracks without gain tags play at their original level.

## Build

Use Rust 1.85 or later, GTK 4.10 or later, GStreamer 1.0, a C linker, pkg-config, and the GLib resource compiler. On Debian 13:

```sh
sudo apt install build-essential pkgconf libglib2.0-dev-bin libgtk-4-dev libgstreamer1.0-dev gstreamer1.0-plugins-base gstreamer1.0-plugins-good
```

Clone the repository, then run Ferritone and its tests.

```sh
git clone https://github.com/ghsantosalves/ferritone.git
cd ferritone
cargo run --locked
cargo test --locked
```

## Install a desktop launcher

Build and install the release binary, desktop entry, and icon for the current user. The desktop entry assumes that the desktop session can find `$HOME/.local/bin` on its executable search path.

```sh
cargo build --release --locked
install -Dm755 target/release/rust-music-player "$HOME/.local/bin/rust-music-player"
install -Dm644 assets/io.github.ghsantosalves.Ferritone.desktop "$HOME/.local/share/applications/io.github.ghsantosalves.Ferritone.desktop"
install -Dm644 assets/player-icon.svg "$HOME/.local/share/icons/hicolor/scalable/apps/io.github.ghsantosalves.Ferritone.svg"
install -Dm644 LICENSE "$HOME/.local/share/doc/ferritone/LICENSE"
```

Open **Ferritone** from the application menu.

Versions before 0.1.4 used the ID `io.github.rustmusicplayer.Player`. If you installed one of them this way, remove the old desktop entry and icon, then pin Ferritone to the dock again:

```sh
rm -f "$HOME/.local/share/applications/io.github.rustmusicplayer.Player.desktop" "$HOME/.local/share/icons/hicolor/scalable/apps/io.github.rustmusicplayer.Player.svg"
```

## Build the Flatpak

With `flatpak-builder` and the Flathub remote, build and install the Flatpak for the current user:

```sh
flatpak-builder --user --install-deps-from=flathub --install --force-clean build-flatpak packaging/flatpak/io.github.ghsantosalves.Ferritone.yml
flatpak run io.github.ghsantosalves.Ferritone
```

The Flatpak reads music anywhere in your home folder but cannot change it, and it keeps its library separately in `~/.var/app/io.github.ghsantosalves.Ferritone/`. After changing dependencies, regenerate the offline crate sources with `python3 packaging/flatpak/cargo-sources.py > packaging/flatpak/cargo-sources.json`.

## Build the Debian package

On Debian 13 amd64, with the build dependencies above plus `dpkg-dev`, run:

```sh
packaging/build-deb.sh
```

The package is written to `target/dist/`. It includes the AppStream metadata, desktop entry, icon, and the license notices of every bundled Rust crate.

To publish a release, update the version in `Cargo.toml` and add a `<release>` entry to `assets/io.github.ghsantosalves.Ferritone.metainfo.xml`, then push a matching tag such as `v0.1.3`. The Release workflow builds the package on Debian 13 and opens a draft GitHub release with the `.deb`, a `.flatpak` bundle, `SHA256SUMS`, and notes taken from the metainfo entry. Run the workflow by hand from the Actions tab to test both builds without publishing. Review the draft and publish it.

## Data and playlists

Ferritone saves the library, playlists, last queue, and preferences in `$XDG_DATA_HOME/rust-music-player/library.json`, which defaults to `~/.local/share/rust-music-player/library.json`. While playing, the position is also saved every 30 seconds to `position.json` in the same folder, which is removed whenever the library is saved. The library references audio files where they are stored. Back up the JSON file to preserve Ferritone's state.

M3U8 exports use relative paths for tracks below the export folder and absolute paths for other tracks. Keep the same folder structure when moving an exported playlist and its audio files together.

## Source layout

| Area | Files |
| --- | --- |
| GTK window, playback controls, and GStreamer pipeline | `src/main.rs` |
| Track skipping and autosave rules | `src/playback.rs` |
| Mini player, waveform rendering, lyrics, and equalizer | `src/mini.rs`, `src/waveform.rs`, `src/lyrics.rs`, `src/equalizer.rs` |
| Library state, tag reading, and playlist import | `src/library.rs`, `src/metadata.rs`, `src/playlist_file.rs` |
| Library visibility controls | `src/library_view.rs` |
| Track columns, themes, and translations | `src/table.rs`, `src/style.css`, `src/i18n/` |
| Track context menu and keyboard shortcuts | `src/track_menu.rs` |
| Desktop media integration | `src/mpris.rs` |
| ReplayGain and peak protection | `src/replay_gain.rs` |
| Debian and Flatpak packaging | `packaging/` |

## License

Copyright © 2026 ghsantosalves.

Ferritone is free software. You may redistribute it or modify it under the terms of the [GNU General Public License, version 3](LICENSE) (`GPL-3.0-only`). Ferritone is distributed without warranty. Third-party dependencies are available under their respective licenses.
