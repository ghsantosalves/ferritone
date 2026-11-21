# Using Ferritone

[Back to the README](../README.md) · [Lyrics](lyrics.md) · [Remote control](remote.md) · [ListenBrainz](listenbrainz.md) · [Building and packaging](building.md)

## Get started

- **Home** shows the first six saved playlists and lets you open a folder, add a folder to the library, or resume the last queue. Search from Home to find a playlist anywhere in the collection. Opening a folder replaces the queue and starts playback. Adding a folder to the library indexes its tracks without changing playback. Both actions include subfolders.
- **Smart lists** on Home play the 100 most recently added tracks in the library, by file date, and, once you have listened to some music, the 100 most recently played and most played tracks. A track counts as played once it passes half its length or four minutes, whichever comes first.
- **Library** lets you add multiple folders and browse or search by title, artist, or album. You can mix local folders and cloud folders mounted or synced to the computer. Ferritone keeps the library in step with its folders: it checks them when it starts and a few seconds after files are added, removed, or renamed while it is open. If a folder is missing, such as an unplugged drive, its tracks stay in the library until it is back. **Manage library** lists the folder roots and lets you hide or show them independently. You can re-add a previously imported folder to register it as a source without duplicating tracks. Right-click selected tracks to hide them from the library. Hidden tracks stay in existing playlists and the queue, and are excluded when starting a new shuffle from the library. Use **View** in the playback menu to choose the visible columns and hide or show the library buttons.
- **Albums** shows the library as a grid of album covers, and **Artists** lists each artist with their album and track counts. Open an album to play it or add it to the queue; its tracks play in disc and track order. Albums are grouped by the album artist tag when present, so compilations stay together.
- **Playlists** supports drag and drop for files and folders, local audio imports from M3U, M3U8, and PLS, and M3U8 export. Search saved playlists or search for tracks inside any open playlist. The active playlist and the currently playing row are marked. Right-click a saved playlist on Home or in Playlists to play it, add it to the queue, add tracks, rename it, export it, or delete it.
- **Queue** shows the playback order and supports track search. Drag a track by its handle or use the arrow buttons to reorder it. Reordering keeps the current track and playback position. Right-click a track to add a copy to the end of the queue or remove the selected entries. The repeat button cycles through off, repeat queue, and repeat track. Tracks follow each other without a gap, so live albums and continuous mixes play as recorded.
- **Track tables** sort when you click a column header. In the queue and playlists, the next click reverses the sort and the third restores the original order. Sorting these views keeps the playback sequence. Drag a header to move the column or drag its divider to resize it. Double-click a divider to fit the column to the tracks in the current list.
- The **playback menu** opens the mini player, the 10-band equalizer with presets, and View settings. Use the mini player's three-dot menu to show album art, live waveforms, and lyrics independently. Ferritone saves the combination and waveform style. The line uses rounded curves. Bars and the mirrored wave respond to the average energy of the audio and recede gradually between beats. The visualizations settle when playback is paused. The main progress slider hides when space is limited. The time display stays visible if enabled.
- **View** lets you search its options and controls the visibility of interface elements, including the sidebar, title bar icon, playlist buttons, and drop area. Playlist controls can be hidden in the overview and in individual playlists. When the overview drop area is hidden, you can still drop files onto the list to create or add to a playlist. Drag the top edge of the playback bar to resize its height or drag the divider next to the track information to resize its width.
- **Preferences** lets you change the theme, color scheme, accent color, and language. The **Ferritone** theme uses charcoal and copper, **System** keeps Ferritone's look in the desktop's light or dark colors with the accent you choose, and **GNOME/Adwaita** leaves every color to libadwaita and the desktop's accent. In a narrow window the sidebar folds away behind a button in the header. Ferritone draws its interface in software by default, which uses far less memory than GTK's GPU renderers for about the same processor time; **Draw the interface with the graphics card** in **Preferences → Audio** switches back after a restart. With **Follow system** as the color scheme or accent, Ferritone follows the desktop's light or dark style and accent color, including when they change while it is open, inside or outside Flatpak. The remote control page uses the same colors. Ferritone follows the system language by default and includes English, Brazilian Portuguese, Spanish, and French. Other locales fall back to English.

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

## Sleep timer

Open **Sleep timer** in the playback menu to pause playback after 15 to 90 minutes or at the end of the current track. A timed sleep lowers the volume gradually over the last 30 seconds, then pauses and restores the volume. Stopping at the end of a track leaves the next track ready, so **Play** continues from there.

## Audio

While music plays, Ferritone keeps the screen from locking and the computer from sleeping. **Preferences → Audio** has a separate option for each, so you can let the screen lock and still keep the music from stopping.

In **Preferences → Audio**, ReplayGain levels volume using embedded gain tags. **Track gain** suits mixed queues, and **Album gain** preserves the relative level of an album and falls back to track gain when needed. The optional peak protection caps boosting using the file's peak tag. Tracks without gain tags play at their original level.

## Desktop media controls

Ferritone exposes track details and playback controls over MPRIS. In GNOME, open the clock and notifications panel to access the current track, play or pause, and skip tracks. Desktop media keys work on environments that route them to MPRIS players.

Right-click Ferritone's icon in the dock or app grid for **Play/Pause**, **Previous Track**, and **Next Track**. If Ferritone is not running, choosing one of them opens it.

To show these controls when you hover over Ferritone's icon in the dock, install the optional GNOME Shell extension in `extras/gnome-shell-dock-controls` (GNOME 48 and 49; works with Dash to Dock and Ubuntu Dock):

```sh
uuid=ferritone-dock-controls@ghsantosalves.github.io
mkdir -p ~/.local/share/gnome-shell/extensions/$uuid
cp extras/gnome-shell-dock-controls/* ~/.local/share/gnome-shell/extensions/$uuid/
```

Log out and back in, then run `gnome-extensions enable $uuid`. The controls use Ferritone's theme and accent color and follow them when you change them in **Preferences**. After updating the extension, log out and back in again so GNOME Shell loads the new version.

## Data and playlists

Ferritone saves the library, playlists, last queue, and preferences in `$XDG_DATA_HOME/rust-music-player/library.json`, which defaults to `~/.local/share/rust-music-player/library.json`. While playing, the position is also saved every 30 seconds to `position.json` in the same folder, which is removed whenever the library is saved. The library references audio files where they are stored. Back up the JSON file to preserve Ferritone's state.

M3U8 exports use relative paths for tracks below the export folder and absolute paths for other tracks. Keep the same folder structure when moving an exported playlist and its audio files together.
