mod album_pages;
mod albums;
mod app;
mod appearance;
mod browser_shortcut;
mod equalizer;
mod files;
mod http;
mod i18n;
mod library;
mod library_view;
mod library_watch;
mod listenbrainz;
mod lyrics;
mod lyrics_view;
mod metadata;
mod mini;
mod mpris;
mod now_playing;
mod pages;
mod playback;
mod playlist_file;
mod remote;
mod remote_ui;
mod replay_gain;
mod scrobble;
mod search;
mod search_ui;
mod settings_ui;
mod sleep_ui;
mod smart_lists;
mod table;
mod track_list;
mod track_menu;
mod transport;
mod waveform;
mod widgets;
mod window;

use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use adw::prelude::*;
use app::*;
use gst::prelude::*;
use gtk::{gdk, gio, glib};
use library::{
    Accent, ColorScheme, Language, Library, Playlist, Repeat, Theme, TrackSort, ViewOptions,
    collect_tracks, data_path, title,
};
use metadata::{TrackInfo, read_cover_image, read_cover_thumbnail, read_track};
use search::{SearchBar, SearchVisibility};
use table::TrackColumns;
use widgets::*;
use window::*;

const APP_NAME: &str = "Ferritone";
const SORT_COLUMNS: [TrackSort; 4] = [
    TrackSort::Title,
    TrackSort::Artist,
    TrackSort::Album,
    TrackSort::Duration,
];
const APP_ID: &str = "io.github.ghsantosalves.Ferritone";

/// Whether the saved preferences ask for GPU drawing. Read before GTK starts, since the
/// renderer cannot change afterwards.
fn wants_gpu_rendering() -> bool {
    std::fs::read(data_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|library| library.get("gpu_rendering")?.as_bool())
        .unwrap_or(true)
}

fn main() -> glib::ExitCode {
    // GTK's GPU renderers keep around 40-150 MB of driver and texture memory, while drawing a
    // music player's interface in software costs about the same CPU. The GPU is the default;
    // software is used when the user turns it off in Preferences, unless GSK_RENDERER is set.
    if std::env::var_os("GSK_RENDERER").is_none() && !wants_gpu_rendering() {
        // SAFETY: this runs first in main, before GTK, GStreamer, or any other thread starts.
        unsafe { std::env::set_var("GSK_RENDERER", "cairo") };
    }
    glib::set_application_name(APP_NAME);
    glib::set_prgname(Some(APP_ID));
    if let Err(error) = gst::init() {
        eprintln!("Could not initialize GStreamer: {error}");
        return glib::ExitCode::FAILURE;
    }
    if let Err(error) = gio::resources_register_include!("ferritone.gresource") {
        eprintln!("Could not register Ferritone resources: {error}");
        return glib::ExitCode::FAILURE;
    }
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(build);
    app.run()
}
