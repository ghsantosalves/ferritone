# Building and packaging

[Back to the README](../README.md)

## Build from source

Use Rust 1.85 or later, GTK 4.10 or later, libadwaita 1.7 or later, GStreamer 1.0, a C linker, pkg-config, and the GLib resource compiler. On Debian 13:

```sh
sudo apt install build-essential pkgconf libglib2.0-dev-bin libgtk-4-dev libadwaita-1-dev libgstreamer1.0-dev gstreamer1.0-plugins-base gstreamer1.0-plugins-good
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
install -Dm755 target/release/ferritone "$HOME/.local/bin/ferritone"
install -Dm644 assets/io.github.ghsantosalves.Ferritone.desktop "$HOME/.local/share/applications/io.github.ghsantosalves.Ferritone.desktop"
install -Dm644 assets/player-icon.svg "$HOME/.local/share/icons/hicolor/scalable/apps/io.github.ghsantosalves.Ferritone.svg"
install -Dm644 LICENSE "$HOME/.local/share/doc/ferritone/LICENSE"
```

Open **Ferritone** from the application menu.

The binary used to be called `rust-music-player`. If you installed an older version this way, remove it after installing the new one; the library in `~/.local/share/rust-music-player/` stays where it is:

```sh
rm -f "$HOME/.local/bin/rust-music-player"
```

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

On Debian 13 amd64, with the [build dependencies](#build-from-source) plus `dpkg-dev`, run:

```sh
packaging/build-deb.sh
```

The package is written to `target/dist/`. It includes the AppStream metadata, desktop entry, icon, and the license notices of every bundled Rust crate.

To publish a release, update the version in `Cargo.toml` and add a `<release>` entry to `assets/io.github.ghsantosalves.Ferritone.metainfo.xml`, then push a matching tag such as `v0.1.3`. The Release workflow builds the package on Debian 13 and opens a draft GitHub release with the `.deb`, a `.flatpak` bundle, `SHA256SUMS`, and notes taken from the metainfo entry. Run the workflow by hand from the Actions tab to test both builds without publishing. Review the draft and publish it.

## Source layout

| Area | Files |
| --- | --- |
| Startup, the `Player` state and page switching, window setup, and shared widgets | `src/main.rs`, `src/app.rs`, `src/window.rs`, `src/widgets.rs` |
| Pages, track tables, and search | `src/pages.rs`, `src/track_list.rs`, `src/search_ui.rs` |
| Albums and artists, smart lists, and folder watching | `src/albums.rs`, `src/album_pages.rs`, `src/smart_lists.rs`, `src/library_watch.rs` |
| Playback, the playback bar, and the current track | `src/transport.rs`, `src/now_playing.rs` |
| Settings windows, file import and export | `src/settings_ui.rs`, `src/files.rs` |
| Track skipping and autosave rules | `src/playback.rs` |
| Mini player, waveform rendering, lyrics, and equalizer | `src/mini.rs`, `src/waveform.rs`, `src/lyrics.rs`, `src/equalizer.rs` |
| Library state, tag reading, and playlist import | `src/library.rs`, `src/metadata.rs`, `src/playlist_file.rs` |
| Library visibility controls | `src/library_view.rs` |
| Track columns, themes, and translations | `src/table.rs`, `src/layout.css`, `src/style.css`, `src/i18n/` |
| Track context menu and keyboard shortcuts | `src/track_menu.rs` |
| Desktop media integration | `src/mpris.rs` |
| ReplayGain and peak protection | `src/replay_gain.rs` |
| Remote control server, page, and window | `src/remote.rs`, `src/remote.html`, `src/remote_ui.rs` |
| HTTPS client for LRCLIB and ListenBrainz | `src/http.rs`, `src/lyrics.rs`, `src/listenbrainz.rs`, `src/scrobble.rs` |
| Debian and Flatpak packaging | `packaging/` |
