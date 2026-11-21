mod album_pages;
mod albums;
mod appearance;
mod browser_shortcut;
mod equalizer;
mod files;
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

use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use gst::prelude::*;
use gtk::{gdk, gio, glib, prelude::*};
use library::{
    Accent, Language, Library, Playlist, Repeat, Theme, TrackSort, ViewOptions, collect_tracks,
    data_path, title,
};
use metadata::{TrackInfo, read_cover_image, read_cover_thumbnail, read_track};
use search::{SearchBar, SearchVisibility};
use table::TrackColumns;

const APP_NAME: &str = "Ferritone";
const SORT_COLUMNS: [TrackSort; 4] = [
    TrackSort::Title,
    TrackSort::Artist,
    TrackSort::Album,
    TrackSort::Duration,
];
type LibraryRefresh = Rc<dyn Fn(&str)>;
type PlaylistIndicator = (usize, glib::WeakRef<gtk::Box>, glib::WeakRef<gtk::Image>);
const APP_ID: &str = "io.github.ghsantosalves.Ferritone";

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Home,
    Library,
    Albums,
    /// The album in `Player::selected_album`.
    Album,
    Artists,
    /// The artist in `Player::selected_artist`.
    Artist,
    Playlists,
    Queue,
    Playlist(usize),
    Preferences,
}

#[derive(Clone, Copy)]
enum Import {
    Play,
    Library,
    Playlist(Option<usize>),
    PickPlaylist,
    ImportPlaylist,
}

struct Player {
    window: gtk::ApplicationWindow,
    app_icon: gtk::Image,
    content: gtk::Box,
    sidebar: gtk::Box,
    /// The sidebar's page buttons, to mark the page on screen.
    sidebar_items: RefCell<Vec<(Page, gtk::Button)>>,
    navigation: gtk::MenuButton,
    columns: RefCell<Option<TrackColumns>>,
    track_rows: RefCell<HashMap<usize, glib::WeakRef<gtk::Box>>>,
    track_highlight: Cell<Option<usize>>,
    playlist_indicators: RefCell<Vec<PlaylistIndicator>>,
    playlist_query: RefCell<(Option<usize>, String)>,
    saved_playlist_query: RefCell<String>,
    playlist_menu_delete: gtk::Button,
    status: gtk::Label,
    /// The status message with its close button.
    status_bar: gtk::Box,
    status_timer: Rc<RefCell<Option<glib::SourceId>>>,
    now_playing: gtk::Label,
    playing_artist: gtk::Label,
    track_caption: gtk::Box,
    lyric_panel: gtk::Box,
    lyric_strip: gtk::CenterBox,
    lyric_lines: [gtk::Label; 5],
    mini_lyrics: gtk::Label,
    lyrics_preference: glib::WeakRef<gtk::CheckButton>,
    online_lyrics_preference: glib::WeakRef<gtk::CheckButton>,
    lyrics: RefCell<lyrics::Lyrics>,
    lyric_request: Cell<u64>,
    lyric_drag_preview: Cell<Option<u64>>,
    online_lyrics_cache: RefCell<HashMap<PathBuf, lyrics::Lyrics>>,
    artwork: gtk::Image,
    thumbnail: RefCell<Option<gtk::gdk_pixbuf::Pixbuf>>,
    thumbnail_size: Cell<i32>,
    compact_search: gtk::Button,
    track_search_field: RefCell<Option<SearchBar>>,
    track_search_open: Cell<SearchVisibility>,
    track_model: glib::WeakRef<gtk::MultiSelection>,
    track_popover: glib::WeakRef<gtk::Popover>,
    library_refresh: RefCell<Option<LibraryRefresh>>,
    library_manager_rows: glib::WeakRef<gtk::Box>,
    local_sort: Cell<Option<(Page, TrackSort, bool)>>,
    queue_query: RefCell<String>,
    artwork_button: gtk::Button,
    cover_source: gtk::Picture,
    mini: RefCell<Option<gtk::ApplicationWindow>>,
    media_session: RefCell<Option<mpris::MediaSession>>,
    waveform: Option<waveform::Waveform>,
    transport: gtk::Box,
    title_grip: gtk::Box,
    volume: gtk::Scale,
    equalizer: Option<gst::Element>,
    replay_gain: Option<replay_gain::ReplayGain>,
    cover: RefCell<Option<gdk::Texture>>,
    artwork_track: RefCell<Option<PathBuf>>,
    info: RefCell<HashMap<PathBuf, TrackInfo>>,
    reading_info: RefCell<HashSet<PathBuf>>,
    library_search: RefCell<String>,
    shuffle: gtk::ToggleButton,
    repeat: gtk::Button,
    css: gtk::CssProvider,
    default_dark: bool,
    desktop_appearance: Option<gio::Settings>,
    /// The desktop's color scheme and accent as the settings portal last reported them.
    portal_color_scheme: Cell<Option<u32>>,
    portal_accent: Cell<Option<&'static str>>,
    /// Keeps the portal signal subscription alive.
    desktop_connection: RefCell<Option<gio::DBusConnection>>,
    toggle: gtk::Button,
    progress: gtk::Scale,
    time: gtk::Label,
    pipeline: gst::Element,
    state: RefCell<Library>,
    storage: Option<PathBuf>,
    page: Cell<Page>,
    active: Cell<bool>,
    playing: Cell<bool>,
    updating_progress: Cell<bool>,
    pending_seek: Cell<u64>,
    pending_seek_signal: Cell<bool>,
    importing: Cell<bool>,
    /// Consecutive tracks that failed to play.
    failures: RefCell<playback::Failures>,
    /// When the playback position is next due to be saved while playing.
    autosave: RefCell<playback::Autosave>,
    /// The next track, handed to GStreamer before the current one ends.
    handoff: Arc<Mutex<playback::Handoff>>,
    /// The phone remote's server while it runs.
    remote: RefCell<Option<remote::Server>>,
    remote_shared: Arc<remote::Shared>,
    remote_sender: mpsc::Sender<remote::Command>,
    remote_receiver: mpsc::Receiver<remote::Command>,
    remote_ticks: Cell<u32>,
    /// What the library list for the phone was built from.
    remote_library_signature: RefCell<Option<(usize, Option<PathBuf>, usize)>>,
    sleep: Cell<playback::SleepTimer>,
    /// The session inhibition that keeps the computer awake while playing; 0 when none.
    inhibit_cookie: Cell<u32>,
    /// Whether the current track's play has been counted.
    play_counted: Cell<bool>,
    /// When the current track started, for ListenBrainz.
    track_started_at: Cell<u64>,
    listens_sending: Cell<bool>,
    /// Whether the ListenBrainz token works, and for which user.
    listenbrainz_state: RefCell<scrobble::Connection>,
    /// Listens ListenBrainz accepted since Ferritone started.
    listens_sent: Cell<usize>,
    listenbrainz_status: RefCell<Option<glib::WeakRef<gtk::Label>>>,
    /// Watches the library folders for added and removed files.
    library_monitors: RefCell<Vec<gio::FileMonitor>>,
    selected_album: RefCell<albums::AlbumKey>,
    selected_artist: RefCell<String>,
    /// Album cover thumbnails by the album's first track; `None` when it has no cover.
    album_covers: RefCell<HashMap<PathBuf, Option<gdk::Texture>>>,
    library_rescan_timer: RefCell<Option<glib::SourceId>>,
    library_scanning: Cell<bool>,
    library_rescan_again: Cell<bool>,
    /// The volume last given to the pipeline, after any sleep fade.
    applied_volume: Cell<f64>,
}

/// Whether the saved preferences ask for GPU drawing. Read before GTK starts, since the
/// renderer cannot change afterwards.
fn wants_gpu_rendering() -> bool {
    std::fs::read(data_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|library| library.get("gpu_rendering")?.as_bool())
        .unwrap_or(false)
}

fn main() -> glib::ExitCode {
    // GTK's GPU renderers keep around 40-150 MB of driver and texture memory, while drawing a
    // music player's interface in software costs about the same CPU. Software is the default
    // unless the user asks for the GPU in Preferences or sets GSK_RENDERER themselves.
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
    let app = gtk::Application::builder().application_id(APP_ID).build();
    app.connect_activate(build);
    app.run()
}

fn build(app: &gtk::Application) {
    if let Some(window) = app.active_window() {
        window.present();
        return;
    }
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title(APP_NAME)
        .icon_name(APP_ID)
        .default_width(1040)
        .default_height(670)
        .build();
    let Ok(pipeline) = gst::ElementFactory::make("playbin").build() else {
        let dialog = gtk::AlertDialog::builder()
            .modal(true)
            .message(Language::System.resolve().text(
                "Install the GStreamer base plugins to start playback.",
                "Instale os plugins básicos do GStreamer para reproduzir áudio.",
            ))
            .build();
        dialog.show(Some(&window));
        window.present();
        return;
    };
    // Keep video and subtitle windows out of the audio player.
    pipeline.set_property_from_str("flags", "audio");
    pipeline.set_property("volume", 0.7f64);
    // Queue the next file before the current one ends so tracks follow each other without a gap.
    let handoff = Arc::new(Mutex::new(playback::Handoff::default()));
    let (remote_sender, remote_receiver) = mpsc::channel();
    let shared = handoff.clone();
    pipeline.connect("about-to-finish", false, move |values| {
        let next = shared.lock().ok().and_then(|mut handoff| handoff.take());
        if let (Some(track), Ok(playbin)) = (next, values[0].get::<gst::Element>()) {
            playbin.set_property("uri", gio::File::for_path(&track).uri().as_str());
        }
        None
    });

    let path = data_path();
    let (mut state, storage, load_error) = match Library::load(&path) {
        Ok(value) => (value, Some(path), None),
        Err(error) => (Library::default(), None, Some(error)),
    };

    state.ensure_shuffle_order();
    let (equalizer, waveform, replay_gain) = waveform::attach(&pipeline);
    if let Some(equalizer) = &equalizer {
        state.equalizer.apply(equalizer);
    }
    if let Some(replay_gain) = &replay_gain {
        replay_gain.apply(state.replay_gain);
    }
    let titlebar = gtk::HeaderBar::new();
    titlebar.add_css_class("compact-titlebar");
    let app_icon = gtk::Image::from_icon_name(APP_ID);
    app_icon.set_pixel_size(27);
    titlebar.pack_start(&app_icon);
    let navigation = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .build();
    navigation.add_css_class("transport-menu");
    window.set_titlebar(Some(&titlebar));

    let layout = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.set_hexpand(true);
    content.set_vexpand(true);
    content.add_css_class("content");

    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.add_css_class("status");
    status.set_hexpand(true);
    let status_bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    status_bar.add_css_class("status-bar");
    status_bar.set_visible(false);
    status_bar.append(&status);
    let close_status = gtk::Button::from_icon_name("window-close-symbolic");
    close_status.add_css_class("flat");
    close_status.set_valign(gtk::Align::Center);
    let bar = status_bar.downgrade();
    close_status.connect_clicked(move |_| {
        if let Some(bar) = bar.upgrade() {
            bar.set_visible(false);
        }
    });
    status_bar.append(&close_status);

    let now_playing = gtk::Label::new(None);
    now_playing.add_css_class("playing-title");
    now_playing.set_xalign(0.0);
    now_playing.set_ellipsize(gtk::pango::EllipsizeMode::End);
    now_playing.set_width_chars(9);
    now_playing.set_max_width_chars(25);
    now_playing.set_wrap(true);
    now_playing.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    now_playing.set_lines(1);
    let playing_artist = gtk::Label::new(None);
    playing_artist.add_css_class("playing-artist");
    playing_artist.set_xalign(0.0);
    playing_artist.set_ellipsize(gtk::pango::EllipsizeMode::End);
    playing_artist.set_max_width_chars(25);
    let track_caption = gtk::Box::new(gtk::Orientation::Vertical, 1);
    track_caption.append(&now_playing);
    track_caption.append(&playing_artist);
    let compact_search = gtk::Button::from_icon_name("system-search-symbolic");
    compact_search.set_visible(false);
    let lyric_panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
    lyric_panel.set_visible(false);
    let lyric_strip = gtk::CenterBox::new();
    lyric_strip.set_orientation(gtk::Orientation::Vertical);
    lyric_strip.add_css_class("transport-lyrics");
    let lyric_group = gtk::Box::new(gtk::Orientation::Vertical, 0);
    lyric_strip.set_center_widget(Some(&lyric_group));
    let lyric_lines = std::array::from_fn(|index| {
        let label = gtk::Label::new(Some(" "));
        label.set_xalign(0.5);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.add_css_class(if index == 2 {
            "lyric-current"
        } else {
            "lyric-nearby"
        });
        if (1..=3).contains(&index) {
            lyric_group.append(&label);
        }
        label
    });
    let mini_lyrics = gtk::Label::new(None);
    let artwork = gtk::Image::from_icon_name("audio-x-generic-symbolic");
    artwork.set_pixel_size(36);
    artwork.set_size_request(36, 36);
    let artwork_button = gtk::Button::new();
    artwork_button.set_child(Some(&artwork));
    artwork_button.add_css_class("artwork-button");
    artwork_button.set_overflow(gtk::Overflow::Hidden);
    artwork_button.set_sensitive(false);
    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 8);
    sidebar.add_css_class("sidebar");
    let shuffle = gtk::ToggleButton::builder()
        .icon_name("media-playlist-shuffle-symbolic")
        .active(state.shuffle)
        .build();
    let repeat = gtk::Button::from_icon_name("media-playlist-repeat-symbolic");
    let css = gtk::CssProvider::new();
    let default_dark = gtk::Settings::default()
        .is_some_and(|settings| settings.is_gtk_application_prefer_dark_theme());
    let desktop_appearance = gio::SettingsSchemaSource::default()
        .and_then(|source| source.lookup("org.gnome.desktop.interface", true))
        .filter(|schema| schema.has_key("color-scheme"))
        .map(|schema| gio::Settings::new_full(&schema, gio::SettingsBackend::NONE, None));
    let toggle = gtk::Button::from_icon_name("media-playback-start-symbolic");
    toggle.add_css_class("play-button");
    let progress = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.1);
    progress.set_draw_value(false);
    progress.set_sensitive(false);
    progress.set_hexpand(true);
    let time = gtk::Label::new(Some("0:00 / 0:00"));
    time.add_css_class("playback-clock");
    time.set_xalign(1.0);
    let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
    volume.set_digits(0);
    volume.set_value_pos(gtk::PositionType::Right);
    volume.set_format_value_func(|_, value| format!("{value:.0}%"));
    volume.set_value(70.0);
    let cover_source = gtk::Picture::new();
    let transport = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    transport.add_css_class("transport");
    let title_grip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    title_grip.add_css_class("column-grip");
    title_grip.set_size_request(8, 26);
    let player = Rc::new(Player {
        window,
        app_icon,
        content,
        sidebar,
        sidebar_items: RefCell::default(),
        navigation,
        columns: RefCell::new(None),
        track_rows: RefCell::new(HashMap::new()),
        track_highlight: Cell::new(None),
        playlist_indicators: RefCell::new(Vec::new()),
        playlist_query: RefCell::new((None, String::new())),
        saved_playlist_query: RefCell::new(String::new()),
        playlist_menu_delete: gtk::Button::new(),
        status,
        status_bar,
        status_timer: Rc::default(),
        now_playing,
        playing_artist,
        track_caption,
        lyric_panel,
        lyric_strip,
        lyric_lines,
        mini_lyrics,
        lyrics_preference: glib::WeakRef::new(),
        online_lyrics_preference: glib::WeakRef::new(),
        lyrics: RefCell::default(),
        lyric_request: Cell::new(0),
        lyric_drag_preview: Cell::new(None),
        online_lyrics_cache: RefCell::default(),
        artwork,
        thumbnail: RefCell::new(None),
        thumbnail_size: Cell::new(36),
        compact_search,
        track_search_field: RefCell::new(None),
        track_search_open: Cell::default(),
        track_model: glib::WeakRef::new(),
        track_popover: glib::WeakRef::new(),
        library_refresh: RefCell::new(None),
        library_manager_rows: glib::WeakRef::new(),
        local_sort: Cell::new(None),
        queue_query: RefCell::new(String::new()),
        artwork_button,
        cover_source,
        mini: RefCell::new(None),
        media_session: RefCell::new(None),
        waveform,
        transport,
        title_grip,
        volume,
        equalizer,
        replay_gain,
        cover: RefCell::new(None),
        artwork_track: RefCell::new(None),
        info: RefCell::new(HashMap::new()),
        reading_info: RefCell::new(HashSet::new()),
        library_search: RefCell::new(String::new()),
        shuffle,
        repeat,
        css,
        default_dark,
        desktop_appearance,
        portal_color_scheme: Cell::new(None),
        portal_accent: Cell::new(None),
        desktop_connection: RefCell::new(None),
        toggle,
        progress,
        time,
        pipeline,
        state: RefCell::new(state),
        storage,
        page: Cell::new(Page::Home),
        active: Cell::new(false),
        playing: Cell::new(false),
        updating_progress: Cell::new(false),
        pending_seek: Cell::new(0),
        pending_seek_signal: Cell::new(false),
        importing: Cell::new(false),
        failures: RefCell::default(),
        autosave: RefCell::new(playback::Autosave::new(Instant::now())),
        handoff,
        remote: RefCell::new(None),
        remote_shared: Arc::default(),
        remote_sender,
        remote_receiver,
        remote_ticks: Cell::new(0),
        remote_library_signature: RefCell::new(None),
        sleep: Cell::new(playback::SleepTimer::Off),
        inhibit_cookie: Cell::new(0),
        play_counted: Cell::new(false),
        track_started_at: Cell::new(0),
        listens_sending: Cell::new(false),
        listenbrainz_state: RefCell::new(scrobble::Connection::Off),
        listens_sent: Cell::new(0),
        listenbrainz_status: RefCell::new(None),
        library_monitors: RefCell::default(),
        selected_album: RefCell::default(),
        selected_artist: RefCell::default(),
        album_covers: RefCell::default(),
        library_rescan_timer: RefCell::new(None),
        library_scanning: Cell::new(false),
        library_rescan_again: Cell::new(false),
        applied_volume: Cell::new(0.7),
    });

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.set_hexpand(true);
    body.set_vexpand(true);
    body.append(&player.status_bar);
    body.append(&player.content);
    top.set_vexpand(true);
    top.append(&player.sidebar);
    top.append(&body);
    layout.append(&top);

    let previous = gtk::Button::from_icon_name("media-skip-backward-symbolic");
    let next = gtk::Button::from_icon_name("media-skip-forward-symbolic");
    for widget in [
        player.artwork_button.clone().upcast::<gtk::Widget>(),
        player.track_caption.clone().upcast(),
        player.title_grip.clone().upcast(),
        player.navigation.clone().upcast(),
        previous.clone().upcast(),
        player.toggle.clone().upcast(),
        next.clone().upcast(),
        player.compact_search.clone().upcast(),
        player.progress.clone().upcast(),
        player.time.clone().upcast(),
        player.shuffle.clone().upcast(),
        player.repeat.clone().upcast(),
        player.volume.clone().upcast(),
    ] {
        widget.set_valign(gtk::Align::Center);
        player.transport.append(&widget);
    }
    let weak = Rc::downgrade(&player);
    player.compact_search.connect_clicked(move |_| {
        if let Some(player) = weak.upgrade() {
            player.toggle_track_search();
        }
    });
    player.install_search_dismissal();
    let resize_strip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    resize_strip.add_css_class("transport-resize");
    resize_strip.set_size_request(-1, 7);
    let lyric_grip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    lyric_grip.add_css_class("transport-resize");
    lyric_grip.set_size_request(-1, 7);
    install_transport_resize(&player, &resize_strip, &lyric_grip);
    player.lyric_panel.append(&lyric_grip);
    player.lyric_panel.append(&player.lyric_strip);
    player.attach_lyric_sync(&player.lyric_strip);
    layout.append(&player.lyric_panel);
    layout.append(&resize_strip);
    layout.append(&player.transport);
    player.window.set_child(Some(&layout));
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &player.css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    player.watch_desktop_appearance();
    player.apply_theme();
    player.apply_view();
    player.accept_files(&player.navigation, Import::PickPlaylist);
    if let Some(appearance) = &player.desktop_appearance {
        let weak = Rc::downgrade(&player);
        appearance.connect_changed(None, move |_, key| {
            if matches!(key, "color-scheme" | "accent-color") {
                if let Some(player) = weak.upgrade() {
                    player.apply_theme();
                }
            }
        });
    }

    let weak = Rc::downgrade(&player);
    player.playlist_menu_delete.connect_clicked(move |_| {
        if let Some(player) = weak.upgrade() {
            if let Page::Playlist(index) = player.page.get() {
                if let Some(popup) = player.navigation.popover() {
                    popup.popdown();
                }
                player.confirm_delete_playlist(index);
            }
        }
    });
    let refresh_navigation = {
        let player = Rc::downgrade(&player);
        let (previous, next) = (previous.clone(), next.clone());
        move || {
            let Some(player) = player.upgrade() else {
                return;
            };
            clear(&player.sidebar);
            let navigation_popup = gtk::Popover::new();
            let navigation_items = gtk::Box::new(gtk::Orientation::Vertical, 4);
            player
                .navigation
                .set_tooltip_text(Some(player.text("Navigation", "Navegação")));
            navigation_popup.add_css_class("player-menu");
            let sections = gtk::Box::new(gtk::Orientation::Vertical, 5);
            player
                .sidebar
                .bind_property("visible", &navigation_items, "visible")
                .invert_boolean()
                .sync_create()
                .build();
            sections.append(&navigation_items);
            let popup = navigation_popup.downgrade();
            sections.append(&button(
                player.text("View", "Visualização"),
                &player,
                move |player| {
                    if let Some(popup) = popup.upgrade() {
                        popup.popdown();
                    }
                    let weak = Rc::downgrade(player);
                    glib::idle_add_local_once(move || {
                        if let Some(player) = weak.upgrade() {
                            player.show_view_settings();
                        }
                    });
                },
            ));
            let popup = navigation_popup.downgrade();
            sections.append(&shortcut_button(
                player.text("Manage library", "Gerenciar biblioteca"),
                "<Primary><Shift>b",
                None,
                player.state.borrow().view.context_menu_keycaps,
                &player,
                move |player| {
                    if let Some(popup) = popup.upgrade() {
                        popup.popdown();
                    }
                    player.show_library_manager();
                },
            ));
            let wordmark = gtk::Label::new(Some(APP_NAME));
            wordmark.add_css_class("wordmark");
            player.sidebar.append(&wordmark);
            player.sidebar_items.borrow_mut().clear();
            for (page, en, pt, icon) in [
                (Page::Home, "Home", "Início", "go-home-symbolic"),
                (
                    Page::Library,
                    "Library",
                    "Biblioteca",
                    "folder-music-symbolic",
                ),
                (Page::Albums, "Albums", "Álbuns", "media-optical-symbolic"),
                (
                    Page::Artists,
                    "Artists",
                    "Artistas",
                    "avatar-default-symbolic",
                ),
                (
                    Page::Playlists,
                    "Playlists",
                    "Listas de reprodução",
                    "view-list-symbolic",
                ),
                (
                    Page::Queue,
                    "Queue",
                    "Fila",
                    "media-playlist-consecutive-symbolic",
                ),
                (
                    Page::Preferences,
                    "Preferences",
                    "Preferências",
                    "preferences-system-symbolic",
                ),
            ] {
                let item = button("", &player, move |player| player.show(page));
                let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                line.append(&gtk::Image::from_icon_name(icon));
                let label = gtk::Label::new(Some(player.text(en, pt)));
                label.set_xalign(0.0);
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                line.append(&label);
                item.set_child(Some(&line));
                item.set_tooltip_text(Some(player.text(en, pt)));
                player.sidebar_items.borrow_mut().push((page, item.clone()));
                if page == Page::Playlists {
                    player.accept_files(&item, Import::PickPlaylist);
                }
                player.sidebar.append(&item);
                let popup = navigation_popup.downgrade();
                navigation_items.append(&button(player.text(en, pt), &player, move |player| {
                    if let Some(popup) = popup.upgrade() {
                        popup.popdown();
                    }
                    player.show(page);
                }));
            }
            for (label, shortcut, action) in [
                (
                    player.text("Open files", "Abrir arquivos"),
                    "<Primary>o",
                    "win.open-files",
                ),
                (
                    player.text("Open folder", "Abrir pasta"),
                    "<Primary>k",
                    "win.open-folder",
                ),
                (
                    player.text("Import playlist", "Importar lista de reprodução"),
                    "<Primary><Shift>o",
                    "win.import-playlist",
                ),
            ] {
                let popup = navigation_popup.downgrade();
                sections.append(&shortcut_button(
                    label,
                    shortcut,
                    None,
                    false,
                    &player,
                    move |player| {
                        if let Some(popup) = popup.upgrade() {
                            popup.popdown();
                        }
                        let _ =
                            gtk::prelude::WidgetExt::activate_action(&player.window, action, None);
                    },
                ));
            }
            let popup = navigation_popup.downgrade();
            sections.append(&shortcut_button(
                player.text("Equalizer", "Equalizador"),
                "<Primary>e",
                None,
                false,
                &player,
                move |player| {
                    if let Some(popup) = popup.upgrade() {
                        popup.popdown();
                    }
                    player.show_equalizer();
                },
            ));
            let popup = navigation_popup.downgrade();
            sections.append(&shortcut_button(
                player.text("Mini player", "Miniplayer"),
                "<Primary>m",
                None,
                false,
                &player,
                move |player| {
                    if let Some(popup) = popup.upgrade() {
                        popup.popdown();
                    }
                    mini::open(player);
                },
            ));
            let popup = navigation_popup.downgrade();
            sections.append(&shortcut_button(
                player.text("Sleep timer", "Timer para desligar"),
                "",
                None,
                false,
                &player,
                move |player| {
                    if let Some(popup) = popup.upgrade() {
                        popup.popdown();
                    }
                    player.show_sleep_timer();
                },
            ));
            let popup = navigation_popup.downgrade();
            sections.append(&shortcut_button(
                player.text("Remote control", "Controle remoto"),
                "",
                None,
                false,
                &player,
                move |player| {
                    if let Some(popup) = popup.upgrade() {
                        popup.popdown();
                    }
                    player.show_remote();
                },
            ));
            if let Some(previous) = player
                .playlist_menu_delete
                .parent()
                .and_downcast::<gtk::Box>()
            {
                previous.remove(&player.playlist_menu_delete);
            }
            player
                .playlist_menu_delete
                .set_label(player.text("Delete playlist", "Excluir lista de reprodução"));
            sections.append(&player.playlist_menu_delete);
            // With the sidebar hidden the menu also lists every page and can be taller than the
            // window; GTK then does not show it at all. Scrolling keeps it within the window.
            let menu_scroll = gtk::ScrolledWindow::builder()
                .child(&sections)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .propagate_natural_height(true)
                .propagate_natural_width(true)
                .build();
            let window = player.window.downgrade();
            let scroll = menu_scroll.downgrade();
            navigation_popup.connect_show(move |_| {
                if let (Some(window), Some(scroll)) = (window.upgrade(), scroll.upgrade()) {
                    scroll.set_max_content_height((window.height() - 120).max(160));
                }
            });
            navigation_popup.set_child(Some(&menu_scroll));
            player.navigation.set_popover(Some(&navigation_popup));
            let artwork_label = player.text("Enlarge album cover", "Ampliar capa do álbum");
            player.artwork_button.set_tooltip_text(Some(artwork_label));
            player
                .artwork_button
                .update_property(&[gtk::accessible::Property::Label(artwork_label)]);
            previous.set_tooltip_text(Some(player.text("Previous track", "Faixa anterior")));
            next.set_tooltip_text(Some(player.text("Next track", "Próxima faixa")));
            player
                .shuffle
                .set_tooltip_text(Some(player.text("Shuffle", "Ordem aleatória")));
            let volume_label = player.text("Volume", "Volume");
            player.volume.set_tooltip_text(Some(volume_label));
            player
                .volume
                .update_property(&[gtk::accessible::Property::Label(volume_label)]);
            player
                .compact_search
                .set_tooltip_text(Some(player.text("Show search", "Mostrar busca")));
            player.title_grip.set_tooltip_text(Some(player.text(
                "Drag to resize track information",
                "Arraste para ajustar a largura da faixa",
            )));
            resize_strip.set_tooltip_text(Some(player.text(
                "Drag to resize playback bar",
                "Arraste para ajustar a altura da barra",
            )));
            player.progress.set_tooltip_text(Some(
                player.text("Playback position", "Posição da reprodução"),
            ));
            player.window.set_title(Some(APP_NAME));
        }
    };
    refresh_navigation();

    let refresh_player = Rc::downgrade(&player);
    // The preferences control emits a window action after saving the language.
    let language_action = gio::SimpleAction::new("refresh-language", None);
    language_action.connect_activate(move |_, _| {
        refresh_navigation();
        if let Some(player) = refresh_player.upgrade() {
            player.update_transport();
            player.update_repeat();
            player.show(Page::Preferences);
        }
    });
    player.window.add_action(&language_action);

    let close = gio::SimpleAction::new("close", None);
    let window = player.window.downgrade();
    close.connect_activate(move |_, _| {
        if let Some(window) = window.upgrade() {
            window.close();
        }
    });
    player.window.add_action(&close);
    app.set_accels_for_action("win.close", &["<Primary>q"]);
    player.install_shortcuts(app);

    // The window holds the player until close; timers and other signals use weak references.
    let keep_alive = RefCell::new(Some(player.clone()));
    player.window.connect_close_request(move |_| {
        if let Some(player) = keep_alive.borrow_mut().take() {
            player.update_position();
            player.save();
            let _ = player.pipeline.set_state(gst::State::Null);
        }
        glib::Propagation::Proceed
    });

    let weak = Rc::downgrade(&player);
    player.toggle.connect_clicked(move |_| {
        if let Some(player) = weak.upgrade() {
            player.toggle();
        }
    });
    let weak = Rc::downgrade(&player);
    player.progress.connect_change_value(move |_, _, seconds| {
        if let Some(player) = weak.upgrade() {
            player.seek(seconds);
        }
        glib::Propagation::Proceed
    });
    let weak = Rc::downgrade(&player);
    previous.connect_clicked(move |_| {
        if let Some(player) = weak.upgrade() {
            player.step(-1);
        }
    });
    let weak = Rc::downgrade(&player);
    next.connect_clicked(move |_| {
        if let Some(player) = weak.upgrade() {
            player.step(1);
        }
    });
    let weak = Rc::downgrade(&player);
    player.volume.connect_value_changed(move |_| {
        if let Some(player) = weak.upgrade() {
            player.apply_volume();
        }
    });

    let weak = Rc::downgrade(&player);
    player.artwork_button.connect_clicked(move |_| {
        if let Some(player) = weak.upgrade() {
            player.show_cover();
        }
    });
    let weak = Rc::downgrade(&player);
    player.shuffle.connect_toggled(move |control| {
        let Some(player) = weak.upgrade() else { return };
        {
            let mut state = player.state.borrow_mut();
            state.shuffle = control.is_active();
            state.reset_shuffle_order();
        }
        player.save();
        if player.page.get() == Page::Queue {
            player.show(Page::Queue);
        }
    });
    let weak = Rc::downgrade(&player);
    player.repeat.connect_clicked(move |_| {
        let Some(player) = weak.upgrade() else { return };
        let mut state = player.state.borrow_mut();
        state.repeat = match state.repeat {
            Repeat::Off => Repeat::Queue,
            Repeat::Queue => Repeat::Track,
            Repeat::Track => Repeat::Off,
        };
        drop(state);
        player.update_repeat();
        player.save();
    });
    let weak = Rc::downgrade(&player);
    glib::timeout_add_local(Duration::from_millis(200), move || {
        let Some(player) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        player.poll();
        glib::ControlFlow::Continue
    });
    let weak = Rc::downgrade(&player);
    glib::timeout_add_local(Duration::from_millis(65), move || {
        let Some(player) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if player.playing.get() && matches!(*player.lyrics.borrow(), lyrics::Lyrics::Timed(_)) {
            player.update_position();
            player.update_lyrics();
        }
        glib::ControlFlow::Continue
    });
    player.show(Page::Home);
    player
        .media_session
        .replace(mpris::MediaSession::attach(&player));
    player.update_repeat();
    player.load_current_track();
    player.update_transport();
    player.rescan_library();
    player.start_listen_retries();
    // The remote page is always available to this computer's browser; the network only when
    // the phone remote is on.
    if player.state.borrow().remote_enabled {
        if let Err(error) = player.start_remote() {
            eprintln!("Could not start the phone remote: {error}");
        }
    } else {
        player.stop_remote();
    }
    if let Some(error) = load_error {
        eprintln!("Could not read library: {error}");
        player.message(player.text(
            "Saved data could not be read. Changes will not be saved in this session.",
            "Não foi possível ler os dados salvos. As alterações desta sessão não serão salvas.",
        ));
    }
    player.window.present();
}

fn install_transport_resize(player: &Rc<Player>, strip: &gtk::Box, lyric_grip: &gtk::Box) {
    #[derive(Clone, Copy)]
    enum Target {
        Lyrics,
        Transport,
        TrackInfo,
    }
    for (handle, target, tooltip) in [
        (
            lyric_grip,
            Target::Lyrics,
            player.text("Resize lyrics", "Redimensionar letras"),
        ),
        (
            strip,
            Target::Transport,
            player.text("Resize playback bar", "Redimensionar barra de reprodução"),
        ),
        (
            &player.title_grip,
            Target::TrackInfo,
            player.text(
                "Resize track information",
                "Redimensionar informações da faixa",
            ),
        ),
    ] {
        let vertical = !matches!(target, Target::TrackInfo);
        handle.set_can_target(true);
        handle.set_cursor_from_name(Some(if vertical { "ns-resize" } else { "ew-resize" }));
        handle.set_tooltip_text(Some(tooltip));
        let start = Rc::new(Cell::new((0, None)));
        let gesture = gtk::GestureDrag::new();
        let weak = Rc::downgrade(player);
        let origin = start.clone();
        let coordinate = move |gesture: &gtk::GestureDrag| {
            gesture
                .current_event()
                .and_then(|event| event.position())
                .map(|(x, y)| if vertical { y } else { x })
        };
        gesture.connect_drag_begin(move |gesture, _, _| {
            if let Some(player) = weak.upgrade() {
                let size = match target {
                    Target::Lyrics => player.lyric_strip.measure(gtk::Orientation::Vertical, -1).0,
                    Target::Transport => player.transport.height(),
                    Target::TrackInfo => player.track_caption.width(),
                };
                origin.set((size, coordinate(gesture)));
            }
        });
        let weak = Rc::downgrade(player);
        let change = move |gesture: &gtk::GestureDrag, dx: f64, dy: f64, finished: bool| {
            let Some(player) = weak.upgrade() else { return };
            let (size, origin) = start.get();
            let delta = coordinate(gesture)
                .zip(origin)
                .map_or(if vertical { dy } else { dx }, |(current, origin)| {
                    current - origin
                }) as i32;
            {
                let mut state = player.state.borrow_mut();
                match target {
                    Target::Lyrics => state.view.lyrics_height = (size - delta).clamp(22, 130),
                    Target::Transport => state.view.transport_height = (size - delta).clamp(1, 130),
                    Target::TrackInfo => {
                        let limit = (player.window.width() / 3).clamp(95, 350);
                        state.view.transport_title_width = (size + delta).clamp(80, limit);
                    }
                }
            }
            if matches!(target, Target::Lyrics) {
                player.update_lyrics();
            } else {
                player.fit_transport();
            }
            if finished {
                player.save();
            }
        };
        let change = Rc::new(change);
        let update = change.clone();
        gesture.connect_drag_update(move |gesture, dx, dy| update(gesture, dx, dy, false));
        gesture.connect_drag_end(move |gesture, dx, dy| change(gesture, dx, dy, true));
        handle.add_controller(gesture);
    }
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn button(
    label: &str,
    player: &Rc<Player>,
    callback: impl Fn(&Rc<Player>) + 'static,
) -> gtk::Button {
    let result = gtk::Button::with_label(label);
    let player = Rc::downgrade(player);
    result.connect_clicked(move |_| {
        if let Some(player) = player.upgrade() {
            callback(&player);
        }
    });
    result
}

fn shortcut_button(
    label: &str,
    accelerator: &str,
    icon: Option<&str>,
    keycaps: bool,
    player: &Rc<Player>,
    callback: impl Fn(&Rc<Player>) + 'static,
) -> gtk::Button {
    let control = button("", player, callback);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    if let Some(icon) = icon {
        let image = gtk::Image::from_icon_name(icon);
        image.set_pixel_size(13);
        content.append(&image);
    }
    let text = gtk::Label::new(Some(label));
    text.set_xalign(0.0);
    text.set_hexpand(true);
    content.append(&text);
    if let Some((key, modifiers)) =
        gtk::accelerator_parse(accelerator).filter(|_| !accelerator.is_empty())
    {
        let shortcut = if keycaps {
            let widget = gtk::ShortcutLabel::new(accelerator);
            widget.add_css_class("menu-shortcut-keys");
            widget.upcast::<gtk::Widget>()
        } else {
            let widget = gtk::Label::new(Some(&gtk::accelerator_get_label(key, modifiers)));
            widget.add_css_class("menu-shortcut");
            widget.upcast::<gtk::Widget>()
        };
        shortcut.set_margin_start(8);
        content.append(&shortcut);
    }
    control.set_child(Some(&content));
    control
}

impl Player {
    fn text<'a>(&self, english: &'a str, portuguese: &'a str) -> &'a str {
        self.state
            .borrow()
            .language
            .resolve()
            .text(english, portuguese)
    }

    /// Shows a message above the page. It can be closed and goes away on its own after a while.
    fn message(&self, message: &str) {
        self.status.set_text(message);
        self.status_bar.set_visible(!message.is_empty());
        if let Some(timer) = self.status_timer.take() {
            timer.remove();
        }
        if message.is_empty() {
            return;
        }
        // Long messages stay a little longer so there is time to read them.
        let seconds = 6 + message.chars().count() as u64 / 25;
        let bar = self.status_bar.downgrade();
        let shown = message.to_owned();
        let label = self.status.downgrade();
        let pending = self.status_timer.clone();
        let timer = glib::timeout_add_local_once(Duration::from_secs(seconds), move || {
            // The source is gone once it has run, so it must not be removed again.
            pending.take();
            let (Some(bar), Some(label)) = (bar.upgrade(), label.upgrade()) else {
                return;
            };
            if label.text() == shown {
                bar.set_visible(false);
            }
        });
        self.status_timer.replace(Some(timer));
    }

    fn save(&self) {
        self.autosave.borrow_mut().saved(Instant::now());
        if let Some(path) = &self.storage {
            if let Err(error) = self.state.borrow().save(path) {
                eprintln!("Could not save library: {error}");
                self.message(self.text(
                    "Changes could not be saved.",
                    "Não foi possível salvar as alterações.",
                ));
            }
        }
    }

    fn save_position(&self) {
        self.autosave.borrow_mut().saved(Instant::now());
        if let Some(path) = &self.storage {
            if let Err(error) = self.state.borrow().save_position(path) {
                eprintln!("Could not save the playback position: {error}");
            }
        }
    }

    fn show(self: &Rc<Self>, page: Page) {
        self.close_track_menu();
        let previous = self.page.replace(page);
        // An album or artist belongs to its list in the sidebar, a playlist to Playlists.
        let section = match page {
            Page::Album => Page::Albums,
            Page::Artist => Page::Artists,
            Page::Playlist(_) => Page::Playlists,
            other => other,
        };
        for (item_page, item) in self.sidebar_items.borrow().iter() {
            if *item_page == section {
                item.add_css_class("active");
            } else {
                item.remove_css_class("active");
            }
        }
        if previous != page
            && self.track_search_open.replace(SearchVisibility::Automatic)
                == SearchVisibility::Shortcut
        {
            match previous {
                Page::Home | Page::Playlists => self.saved_playlist_query.borrow_mut().clear(),
                Page::Library => self.library_search.borrow_mut().clear(),
                Page::Queue => self.queue_query.borrow_mut().clear(),
                Page::Playlist(_) => self.playlist_query.borrow_mut().1.clear(),
                Page::Preferences | Page::Albums | Page::Album | Page::Artists | Page::Artist => {}
            }
        }
        self.columns.replace(None);
        self.track_rows.borrow_mut().clear();
        self.track_highlight.set(None);
        self.playlist_indicators.borrow_mut().clear();
        self.playlist_menu_delete.set_visible(
            matches!(page, Page::Playlist(_)) && !self.state.borrow().view.playlist_actions,
        );
        self.track_search_field.replace(None);
        self.track_model.set(None);
        self.library_refresh.replace(None);
        self.window.set_title(Some(APP_NAME));
        clear(&self.content);
        match page {
            Page::Home => self.home(),
            Page::Library => self.library(),
            Page::Albums => self.albums_page(),
            Page::Album => self.album_page(),
            Page::Artists => self.artists_page(),
            Page::Artist => self.artist_page(),
            Page::Playlists => self.playlists(),
            Page::Queue => self.queue(),
            Page::Playlist(index) => self.playlist(index),
            Page::Preferences => self.preferences(),
        }
        self.fit_search();
    }

    fn heading(&self, title: &str) {
        if self.state.borrow().view.titlebar_heading {
            self.window
                .set_title(Some(&format!("{} · {title}", APP_NAME)));
            return;
        }
        let label = gtk::Label::new(Some(title));
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.add_css_class("page-title");
        self.content.append(&label);
    }

    fn hint(&self, text: &str) {
        let label = gtk::Label::new(Some(text));
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.add_css_class("muted");
        self.content.append(&label);
    }
}

fn sort_tracks(
    tracks: &mut [(usize, &PathBuf)],
    metadata: &HashMap<PathBuf, TrackInfo>,
    sort: TrackSort,
    descending: bool,
) {
    let key = |(_, path): &(usize, &PathBuf)| {
        let info = metadata.get(*path);
        let name = info
            .and_then(|info| info.title.clone())
            .unwrap_or_else(|| title(path))
            .to_lowercase();
        let text = match sort {
            TrackSort::Title => Some(name.clone()),
            TrackSort::Artist => info
                .and_then(|info| info.artist.as_deref())
                .map(str::to_lowercase),
            TrackSort::Album => info
                .and_then(|info| info.album.as_deref())
                .map(str::to_lowercase),
            TrackSort::Duration => None,
        };
        let duration = if sort == TrackSort::Duration {
            info.and_then(|info| info.duration_ms)
        } else {
            None
        };
        let key = text.as_deref().map(library_collation_key);
        (
            key.is_none() && duration.is_none(),
            (key, duration),
            library_collation_key(&name),
        )
    };
    if descending {
        tracks.sort_by_cached_key(|path| {
            let (missing, value, title) = key(path);
            (missing, std::cmp::Reverse(value), title)
        });
    } else {
        tracks.sort_by_cached_key(key);
    }
}

fn replace_indexes(model: &gtk::StringList, indexes: impl IntoIterator<Item = usize>) {
    let indexes: Vec<_> = indexes.into_iter().map(|index| index.to_string()).collect();
    let items: Vec<_> = indexes.iter().map(String::as_str).collect();
    model.splice(0, model.n_items(), &items);
}

fn track_matches(path: &std::path::Path, info: Option<&TrackInfo>, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let matches = |text: &str| search::search_key(text).contains(query);
    matches(&title(path))
        || info.is_some_and(|info| {
            [&info.title, &info.artist, &info.album]
                .into_iter()
                .flatten()
                .any(|text| matches(text))
        })
}

fn library_collation_key(value: &str) -> glib::FilenameCollationKey {
    let normalized = value.trim_start_matches(|character: char| !character.is_alphanumeric());
    glib::FilenameCollationKey::from(if normalized.is_empty() {
        value
    } else {
        normalized
    })
}

fn set_visible(widget: &impl IsA<gtk::Widget>, visible: bool) {
    if widget.get_visible() != visible {
        widget.set_visible(visible);
    }
}

fn scale_image(pixbuf: &gtk::gdk_pixbuf::Pixbuf, side: i32) -> Option<gtk::gdk_pixbuf::Pixbuf> {
    let scale = f64::from(side) / f64::from(pixbuf.width().max(pixbuf.height()));
    let width = (f64::from(pixbuf.width()) * scale).round().max(1.0) as i32;
    let height = (f64::from(pixbuf.height()) * scale).round().max(1.0) as i32;
    pixbuf.scale_simple(width, height, gtk::gdk_pixbuf::InterpType::Bilinear)
}

fn timestamp(milliseconds: u64) -> String {
    let seconds = milliseconds / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn icon_button(
    icon: &str,
    tooltip: &str,
    player: &Rc<Player>,
    callback: impl Fn(&Rc<Player>) + 'static,
) -> gtk::Button {
    let control = button("", player, callback);
    control.set_icon_name(icon);
    control.set_tooltip_text(Some(tooltip));
    control.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    control.add_css_class("row-action");
    control
}

fn track_cell_text<'a>(
    path: &std::path::Path,
    info: Option<&'a TrackInfo>,
    index: usize,
) -> Cow<'a, str> {
    match index {
        0 => info
            .and_then(|info| info.title.as_deref())
            .map_or_else(|| Cow::Owned(title(path)), Cow::Borrowed),
        1 => Cow::Borrowed(info.and_then(|info| info.artist.as_deref()).unwrap_or("—")),
        2 => Cow::Borrowed(info.and_then(|info| info.album.as_deref()).unwrap_or("—")),
        _ => info
            .and_then(|info| info.duration_ms)
            .map_or(Cow::Borrowed("—"), |duration| {
                Cow::Owned(timestamp(duration))
            }),
    }
}

fn table_label(text: &str, width: i32) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_width_request(width);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    if !text.is_empty() {
        label.set_has_tooltip(true);
        label.connect_query_tooltip(|label, _, _, _, tooltip| {
            if !label.layout().is_ellipsized() {
                return false;
            }
            tooltip.set_text(Some(&label.text()));
            true
        });
    }
    label
}

fn style_row(row: &impl IsA<gtk::Widget>, position: usize) {
    row.add_css_class("table-row");
    if position % 2 == 1 {
        row.add_css_class("alternate");
    }
}
