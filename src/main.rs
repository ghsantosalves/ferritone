mod equalizer;
mod i18n;
mod library;
mod library_view;
mod lyrics;
mod lyrics_view;
mod metadata;
mod mini;
mod mpris;
mod playlist_file;
mod replay_gain;
mod search;
mod table;
mod track_menu;
mod waveform;

use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

use gst::prelude::*;
use gtk::{gdk, gio, glib, prelude::*};
use library::{
    Accent, Language, Library, Playlist, Repeat, Theme, TrackSort, ViewOptions, collect_tracks,
    data_path, title,
};
use metadata::{TrackInfo, read_cover_image, read_track};
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
const APP_ID: &str = "io.github.rustmusicplayer.Player";

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Home,
    Library,
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
    navigation: gtk::MenuButton,
    columns: RefCell<Option<TrackColumns>>,
    track_rows: RefCell<HashMap<usize, glib::WeakRef<gtk::Box>>>,
    track_highlight: Cell<Option<usize>>,
    playlist_indicators: RefCell<Vec<PlaylistIndicator>>,
    playlist_query: RefCell<(Option<usize>, String)>,
    saved_playlist_query: RefCell<String>,
    playlist_menu_delete: gtk::Button,
    status: gtk::Label,
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
}

fn main() -> glib::ExitCode {
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
    status.set_visible(false);

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
        navigation,
        columns: RefCell::new(None),
        track_rows: RefCell::new(HashMap::new()),
        track_highlight: Cell::new(None),
        playlist_indicators: RefCell::new(Vec::new()),
        playlist_query: RefCell::new((None, String::new())),
        saved_playlist_query: RefCell::new(String::new()),
        playlist_menu_delete: gtk::Button::new(),
        status,
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
    });

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.set_hexpand(true);
    body.set_vexpand(true);
    body.append(&player.status);
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
            for (page, en, pt) in [
                (Page::Home, "Home", "Início"),
                (Page::Library, "Library", "Biblioteca"),
                (Page::Playlists, "Playlists", "Listas de reprodução"),
                (Page::Queue, "Queue", "Fila"),
                (Page::Preferences, "Preferences", "Preferências"),
            ] {
                let item = button(player.text(en, pt), &player, move |player| {
                    player.show(page)
                });
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
            navigation_popup.set_child(Some(&sections));
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
    player.volume.connect_value_changed(move |slider| {
        if let Some(player) = weak.upgrade() {
            player
                .pipeline
                .set_property("volume", slider.value() / 100.0);
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

    fn message(&self, message: &str) {
        self.status.set_text(message);
        self.status.set_visible(!message.is_empty());
    }

    fn save(&self) {
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

    fn show(self: &Rc<Self>, page: Page) {
        self.close_track_menu();
        let previous = self.page.replace(page);
        if previous != page
            && self.track_search_open.replace(SearchVisibility::Automatic)
                == SearchVisibility::Shortcut
        {
            match previous {
                Page::Home | Page::Playlists => self.saved_playlist_query.borrow_mut().clear(),
                Page::Library => self.library_search.borrow_mut().clear(),
                Page::Queue => self.queue_query.borrow_mut().clear(),
                Page::Playlist(_) => self.playlist_query.borrow_mut().1.clear(),
                Page::Preferences => {}
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
            Page::Playlists => self.playlists(),
            Page::Queue => self.queue(),
            Page::Playlist(index) => self.playlist(index),
            Page::Preferences => self.preferences(),
        }
        self.fit_search();
    }

    fn search_bar(&self, entry: &gtk::SearchEntry) -> SearchBar {
        SearchBar::new(
            entry,
            self.text("Previous result", "Resultado anterior"),
            self.text("Next result", "Próximo resultado"),
        )
    }

    fn mount_track_search(&self, entry: &gtk::SearchEntry) -> SearchBar {
        let bar = self.search_bar(entry);
        self.content.append(&bar.widget);
        self.track_search_field.replace(Some(bar.clone()));
        bar
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

    fn home(self: &Rc<Self>) {
        self.heading(self.text("Home", "Início"));
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.add_css_class("home-actions");
        for (label, tooltip, action) in [
            (
                self.text("Open folder", "Abrir pasta"),
                self.text(
                    "Play music from a folder now",
                    "Tocar agora as músicas de uma pasta",
                ),
                Import::Play,
            ),
            (
                self.text("Add folder to library", "Adicionar pasta à biblioteca"),
                self.text(
                    "Add music to the library without changing playback",
                    "Adicionar músicas à biblioteca sem alterar a reprodução",
                ),
                Import::Library,
            ),
        ] {
            let control = button(label, self, move |player| player.choose_folder(action));
            control.set_tooltip_text(Some(tooltip));
            if matches!(action, Import::Play) {
                control.add_css_class("suggested-action");
            }
            actions.append(&control);
        }
        if !self.state.borrow().queue.is_empty() {
            actions.append(&button(
                self.text("Resume queue", "Retomar fila"),
                self,
                |player| {
                    if !player.playing.get() {
                        player.toggle();
                    }
                    player.show(Page::Queue);
                },
            ));
        }
        self.content.append(&actions);

        self.new_playlist_drop_zone();
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let label = gtk::Label::new(Some(
            self.text("Saved playlists", "Listas de reprodução salvas"),
        ));
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.add_css_class("section-title");
        header.append(&label);
        header.append(&button(
            self.text("View all", "Ver todas"),
            self,
            |player| player.show(Page::Playlists),
        ));
        self.content.append(&header);

        self.content.append(&self.playlist_picker(None));
    }

    fn new_playlist_drop_zone(self: &Rc<Self>) {
        let new = button(
            self.text(
                "New playlist · Drop music or playlist files",
                "Nova lista de reprodução · Solte músicas ou listas aqui",
            ),
            self,
            |player| player.ask_name(None, Vec::new()),
        );
        new.add_css_class("drop-zone");
        if let Some(label) = new.child().and_downcast::<gtk::Label>() {
            label.set_wrap(true);
            label.set_justify(gtk::Justification::Center);
        }
        self.accept_files(&new, Import::Playlist(None));
        self.content.append(&new);
    }

    fn playlists(self: &Rc<Self>) {
        self.heading(self.text("Playlists", "Listas de reprodução"));
        let view = self.state.borrow().view;
        if view.playlist_actions {
            let import = button(
                self.text("Import playlist", "Importar lista de reprodução"),
                self,
                |player| player.choose_files(Import::ImportPlaylist),
            );
            import.add_css_class("flat");
            import.set_halign(gtk::Align::Start);
            self.content.append(&import);
        }
        let picker = self.playlist_picker(None);
        if view.playlist_drop_zone {
            self.new_playlist_drop_zone();
        } else {
            self.accept_files(&picker, Import::Playlist(None));
        }
        self.content.append(&picker);
    }

    fn playlist_entries(&self) -> Vec<(usize, String, usize)> {
        let mut entries: Vec<_> = self
            .state
            .borrow()
            .playlists
            .iter()
            .enumerate()
            .map(|(index, list)| (index, list.name.clone(), list.tracks.len()))
            .collect();
        entries.sort_by_cached_key(|(_, name, _)| name.to_lowercase());
        entries
    }

    fn playlist_picker(
        self: &Rc<Self>,
        destination: Option<(Rc<Vec<PathBuf>>, glib::WeakRef<gtk::Window>)>,
    ) -> gtk::Box {
        let container = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some(
            self.text("Search playlists", "Buscar listas de reprodução"),
        ));
        let bar = self.search_bar(&search);
        let main_page = destination.is_none();
        if main_page {
            search.set_text(&self.saved_playlist_query.borrow());
            self.track_search_field.replace(Some(bar.clone()));
        }
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        scroll.set_child(Some(&rows));
        container.append(&bar.widget);
        container.append(&scroll);
        let entries = self.playlist_entries();
        let home = main_page && self.page.get() == Page::Home;
        let weak = Rc::downgrade(self);
        bar.results.playlist_rows(&rows, &scroll);
        let results = bar.results;
        let update = move |query: &str| {
            let Some(player) = weak.upgrade() else { return };
            if destination.is_none() {
                player.close_track_menu();
                player.playlist_indicators.borrow_mut().clear();
            }
            clear(&rows);
            let query = search::search_key(query);
            let limit = if home && query.is_empty() {
                6
            } else {
                usize::MAX
            };
            let mut matches = 0;
            for (position, (index, name, count)) in entries
                .iter()
                .filter(|(_, name, _)| search::search_key(name).contains(&query))
                .take(limit)
                .enumerate()
            {
                matches += 1;
                rows.append(&player.playlist_item(
                    *index,
                    name,
                    *count,
                    position,
                    destination.clone(),
                ));
            }
            if rows.first_child().is_none() {
                let empty = gtk::Label::new(Some(player.text(
                    "No playlists found",
                    "Nenhuma lista de reprodução encontrada",
                )));
                empty.add_css_class("empty-state");
                rows.append(&empty);
            }
            results.update(&query, matches);
        };
        update(search.text().as_str());
        let weak = Rc::downgrade(self);
        search.connect_search_changed(move |field| {
            if main_page {
                if let Some(player) = weak.upgrade() {
                    player
                        .saved_playlist_query
                        .replace(field.text().to_string());
                }
            }
            update(field.text().as_str());
        });
        container
    }

    fn playlist_item(
        self: &Rc<Self>,
        index: usize,
        name: &str,
        count: usize,
        position: usize,
        destination: Option<(Rc<Vec<PathBuf>>, glib::WeakRef<gtk::Window>)>,
    ) -> gtk::Box {
        let picking = destination.is_some();
        let item = button("", self, move |player| {
            if let Some((tracks, dialog)) = &destination {
                if let Some(dialog) = dialog.upgrade() {
                    dialog.close();
                }
                player.add_to_playlist(index, tracks.as_ref().clone());
            } else {
                player.show(Page::Playlist(index));
            }
        });
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let indicator = gtk::Image::from_icon_name("view-list-symbolic");
        line.append(&indicator);
        let name_label = table_label(name, 0);
        name_label.set_hexpand(true);
        line.append(&name_label);
        let unit = if count == 1 {
            self.text("track", "faixa")
        } else {
            self.text("tracks", "faixas")
        };
        line.append(&gtk::Label::new(Some(&format!("{count} {unit}"))));
        item.set_child(Some(&line));
        item.set_hexpand(true);
        item.add_css_class("track-title");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        style_row(&row, position);
        row.append(&item);
        if !picking {
            self.attach_playlist_menu(&row, index);
            self.playlist_indicators.borrow_mut().push((
                index,
                row.downgrade(),
                indicator.downgrade(),
            ));
            self.style_playlist_indicator(index, &row, &indicator);
            self.accept_files(&item, Import::Playlist(Some(index)));
            if self.page.get() == Page::Playlists {
                row.append(&self.delete_playlist_button(index));
            }
        }
        row
    }

    fn style_playlist_indicator(&self, index: usize, row: &gtk::Box, indicator: &gtk::Image) {
        let active = self.active.get() && self.state.borrow().queue_playlist == Some(index);
        if active {
            row.add_css_class("current");
            indicator.set_icon_name(Some("media-playback-start-symbolic"));
            indicator.set_tooltip_text(Some(self.text("Playing playlist", "Lista em reprodução")));
        } else {
            row.remove_css_class("current");
            indicator.set_icon_name(Some("view-list-symbolic"));
            indicator.set_tooltip_text(None);
        }
    }

    fn refresh_playing_playlist(&self) {
        for (index, row, indicator) in self.playlist_indicators.borrow().iter() {
            if let (Some(row), Some(indicator)) = (row.upgrade(), indicator.upgrade()) {
                self.style_playlist_indicator(*index, &row, &indicator);
            }
        }
    }

    fn delete_playlist_button(self: &Rc<Self>, index: usize) -> gtk::Button {
        icon_button(
            "user-trash-symbolic",
            self.text("Delete playlist", "Excluir lista de reprodução"),
            self,
            move |player| player.confirm_delete_playlist(index),
        )
    }

    fn confirm_delete_playlist(self: &Rc<Self>, index: usize) {
        let name = {
            let state = self.state.borrow();
            let Some(playlist) = state.playlists.get(index) else {
                return;
            };
            playlist.name.clone()
        };
        let detail = format!(
            "{name}\n\n{}",
            self.text(
                "Only the saved playlist will be removed. The audio files will be kept.",
                "Só a lista de reprodução salva será removida. Os arquivos de áudio serão mantidos."
            )
        );
        let dialog = gtk::AlertDialog::builder()
            .message(self.text("Delete playlist?", "Excluir lista de reprodução?"))
            .detail(&detail)
            .buttons([
                self.text("Cancel", "Cancelar"),
                self.text("Delete", "Excluir"),
            ])
            .cancel_button(0)
            .default_button(0)
            .modal(true)
            .build();
        let weak = Rc::downgrade(self);
        dialog.choose(Some(&self.window), gio::Cancellable::NONE, move |choice| {
            let (Some(player), Ok(1)) = (weak.upgrade(), choice) else {
                return;
            };
            let mut state = player.state.borrow_mut();
            if state
                .playlists
                .get(index)
                .is_none_or(|playlist| playlist.name != name)
            {
                return;
            }
            state.remove_playlist(index);
            drop(state);
            player.local_sort.set(None);
            player.playlist_query.replace((None, String::new()));
            player.show(Page::Playlists);
            player.save();
        });
    }

    fn choose_playlist(self: &Rc<Self>, tracks: Vec<PathBuf>) {
        let dialog = gtk::Window::builder()
            .transient_for(&self.window)
            .modal(true)
            .title(self.text("Add to playlist", "Adicionar à lista de reprodução"))
            .default_width(460)
            .default_height(420)
            .build();
        let form = gtk::Box::new(gtk::Orientation::Vertical, 10);
        form.add_css_class("content");
        let tracks = Rc::new(tracks);
        let staged = tracks.clone();
        let window = dialog.downgrade();
        form.append(&button(
            self.text("New playlist", "Nova lista de reprodução"),
            self,
            move |player| {
                if let Some(window) = window.upgrade() {
                    window.close();
                }
                player.ask_name(None, staged.as_ref().clone());
            },
        ));
        form.append(&self.playlist_picker(Some((tracks, dialog.downgrade()))));
        dialog.set_child(Some(&form));
        dialog.present();
    }

    fn library(self: &Rc<Self>) {
        self.heading(self.text("Library", "Biblioteca"));
        if self.state.borrow().view.library_actions {
            self.content.append(&self.library_toolbar());
        }
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some(self.text(
            "Search tracks, artists and albums",
            "Buscar faixas, artistas e álbuns",
        )));
        search.set_text(&self.library_search.borrow());
        let bar = self.mount_track_search(&search);
        let viewport = self.track_viewport(false, 1);
        let library = Rc::new(self.state.borrow().tracks.clone());
        let visible = {
            let state = self.state.borrow();
            library
                .iter()
                .filter(|track| state.library_track_visible(track))
                .cloned()
                .collect::<Vec<_>>()
        };
        self.scan_metadata(&visible);
        let strings = gtk::StringList::new(&[]);
        let paths = library.clone();
        let playback = paths.clone();
        let list = self.virtual_tracks(
            &strings,
            move |player, index, position| {
                paths
                    .get(index)
                    .map(|path| player.library_row(path, position))
            },
            move |player, index| {
                if let Some(path) = playback.get(index) {
                    player.play_library_track(path);
                }
            },
        );
        let empty = gtk::Label::new(Some(
            self.text("No tracks found", "Nenhuma faixa encontrada"),
        ));
        empty.add_css_class("empty-state");
        empty.set_valign(gtk::Align::Start);
        self.content.append(&empty);
        viewport.set_child(Some(&list));
        bar.results.track_list(&list);
        let results = bar.results;
        let weak = Rc::downgrade(self);
        let update = move |query: &str| {
            let Some(player) = weak.upgrade() else { return };
            let query = search::search_key(query);
            let metadata = player.info.borrow();
            let state = player.state.borrow();
            let mut tracks: Vec<_> = library
                .iter()
                .enumerate()
                .filter(|(_, path)| {
                    state.library_track_visible(path)
                        && track_matches(path, metadata.get(*path), &query)
                })
                .collect();
            let (sort, descending) = (state.sort, state.descending);
            drop(state);
            sort_tracks(&mut tracks, &metadata, sort, descending);
            drop(metadata);
            let has_results = !tracks.is_empty();
            replace_indexes(&strings, tracks.into_iter().map(|(index, _)| index));
            set_visible(&empty, !has_results);
            set_visible(&viewport, has_results);
            results.update(&query, strings.n_items());
        };
        let update: LibraryRefresh = Rc::new(update);
        self.library_refresh.replace(Some(update.clone()));
        update(search.text().as_str());
        let weak = Rc::downgrade(self);
        let update_search = update.clone();
        search.connect_search_changed(move |search| {
            if let Some(player) = weak.upgrade() {
                player.library_search.replace(search.text().to_string());
                update_search(search.text().as_str());
            }
        });
    }

    fn library_row(self: &Rc<Self>, path: &PathBuf, position: usize) -> (gtk::Box, gtk::Box) {
        let (row, track) = self.track_row(path, position);
        let drag = gtk::DragSource::builder()
            .actions(gdk::DragAction::COPY)
            .build();
        let file = gio::File::for_path(path);
        drag.connect_prepare(move |_, _, _| {
            let files = gdk::FileList::from_array(std::slice::from_ref(&file));
            Some(gdk::ContentProvider::for_value(&files.to_value()))
        });
        track.add_controller(drag);
        let source = path.clone();
        self.track_actions(&row).append(&icon_button(
            "list-add-symbolic",
            self.text("Add to playlist", "Adicionar à lista de reprodução"),
            self,
            move |player| player.choose_playlist(vec![source.clone()]),
        ));
        (row, track)
    }

    fn sort_column(self: &Rc<Self>, column: usize) {
        let page = self.page.get();
        if page == Page::Library {
            self.sort_library(column);
            return;
        }
        let sort = SORT_COLUMNS[column];
        let next = match self.local_sort.get() {
            Some((same, previous, false)) if same == page && previous == sort => {
                Some((page, sort, true))
            }
            Some((same, previous, true)) if same == page && previous == sort => None,
            _ => Some((page, sort, false)),
        };
        self.local_sort.set(next);
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(player) = weak.upgrade() {
                if player.page.get() == page {
                    player.show(page);
                }
            }
        });
    }

    fn local_track_sort(&self) -> Option<(TrackSort, bool)> {
        self.local_sort
            .get()
            .filter(|(page, _, _)| *page == self.page.get())
            .map(|(_, sort, reverse)| (sort, reverse))
    }

    fn filtered_indexes(
        &self,
        tracks: &[PathBuf],
        order: impl IntoIterator<Item = usize>,
        query: &str,
    ) -> Vec<usize> {
        let query = search::search_key(query);
        let tags = self.info.borrow();
        let mut rows: Vec<_> = order
            .into_iter()
            .filter_map(|index| {
                let path = tracks.get(index)?;
                track_matches(path, tags.get(path), &query).then_some((index, path))
            })
            .collect();
        if let Some((sort, descending)) = self.local_track_sort() {
            sort_tracks(&mut rows, &tags, sort, descending);
        }
        rows.into_iter().map(|(index, _)| index).collect()
    }

    fn sort_library(&self, column: usize) {
        let sort = SORT_COLUMNS[column];
        let descending = {
            let mut state = self.state.borrow_mut();
            state.descending = state.sort == sort && !state.descending;
            state.sort = sort;
            state.descending
        };
        self.save();
        if let Some(columns) = self.columns.borrow().as_ref() {
            for index in 0..4 {
                columns.set_title(index, &self.column_title(index, true, sort, descending));
            }
        }
        if let Some(refresh) = self.library_refresh.borrow().as_ref() {
            refresh(&self.library_search.borrow());
        }
    }

    fn column_title(
        &self,
        index: usize,
        library: bool,
        sort: TrackSort,
        descending: bool,
    ) -> String {
        let labels = [
            self.text("Title", "Título"),
            self.text("Artist", "Artista"),
            self.text("Album", "Álbum"),
            self.text("Length", "Duração"),
        ];
        if library && SORT_COLUMNS[index] == sort {
            format!("{} {}", labels[index], if descending { "↓" } else { "↑" })
        } else {
            labels[index].to_owned()
        }
    }

    fn timeline_fits(&self) -> bool {
        const MIN_TIMELINE_WIDTH: i32 = 100;
        let window_width = self.window.width();
        if matches!(window_width, 1..=760) {
            return false;
        }

        let width = self.transport.width();
        if width == 0 {
            return true;
        }

        let mut needed = MIN_TIMELINE_WIDTH;
        let mut child = self.transport.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if widget.get_visible() && widget != self.progress {
                let width = widget.measure(gtk::Orientation::Horizontal, -1).1;
                needed += width + self.transport.spacing();
            }
        }
        width >= needed
    }

    fn toggle_track_search(&self) {
        let Some(bar) = self.track_search_field.borrow().clone() else {
            return;
        };
        if bar.widget.get_visible() {
            self.collapse_search(&bar);
        } else {
            self.track_search_open.set(SearchVisibility::Expanded);
            self.fit_search();
            bar.entry.grab_focus();
        }
    }

    fn open_search_shortcut(&self) {
        let search = self.track_search_field.borrow().clone();
        if let Some(bar) = search {
            self.track_search_open.set(SearchVisibility::Shortcut);
            self.fit_search();
            bar.entry.grab_focus();
        }
    }

    fn collapse_search(&self, bar: &SearchBar) {
        self.track_search_open.set(SearchVisibility::Collapsed);
        bar.entry.set_text("");
        self.fit_search();
    }

    fn install_search_dismissal(self: &Rc<Self>) {
        let events = gtk::EventControllerLegacy::new();
        events.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        events.connect_event(move |_, event| {
            if event.event_type() != gdk::EventType::ButtonRelease {
                return glib::Propagation::Proceed;
            }
            let Some(player) = weak
                .upgrade()
                .filter(|p| p.track_search_open.get() == SearchVisibility::Shortcut)
            else {
                return glib::Propagation::Proceed;
            };
            let Some(bar) = player.track_search_field.borrow().clone() else {
                return glib::Propagation::Proceed;
            };
            let Some((x, y)) = event.position() else {
                return glib::Propagation::Proceed;
            };
            if player
                .window
                .pick(x, y, gtk::PickFlags::DEFAULT)
                .is_some_and(|picked| {
                    picked == bar.widget
                        || picked.is_ancestor(&bar.widget)
                        || picked == player.compact_search
                        || picked.is_ancestor(&player.compact_search)
                })
            {
                return glib::Propagation::Proceed;
            }
            let weak = weak.clone();
            let entry = bar.entry.downgrade();
            glib::idle_add_local_once(move || {
                let (Some(player), Some(entry)) = (weak.upgrade(), entry.upgrade()) else {
                    return;
                };
                let current = player.track_search_field.borrow().clone();
                if let Some(bar) = current.filter(|bar| bar.entry == entry) {
                    if player.track_search_open.get() == SearchVisibility::Shortcut {
                        player.collapse_search(&bar);
                    }
                }
            });
            glib::Propagation::Proceed
        });
        self.window.add_controller(events);
    }

    fn fit_search(&self) {
        let view = self.state.borrow().view;
        let search = self.track_search_field.borrow();
        let mut show_button = false;
        if let Some(bar) = search.as_ref() {
            let visible = match self.track_search_open.get() {
                SearchVisibility::Automatic => !view.search_button || !bar.entry.text().is_empty(),
                SearchVisibility::Expanded | SearchVisibility::Shortcut => true,
                SearchVisibility::Collapsed => false,
            };
            set_visible(&bar.widget, visible);
            show_button = view.search_button || !visible;
        }
        set_visible(&self.compact_search, show_button);
        set_visible(&self.progress, view.seek_bar && self.timeline_fits());
    }

    fn queue(self: &Rc<Self>) {
        self.heading(self.text("Queue", "Fila"));
        let (tracks, order) = {
            let state = self.state.borrow();
            let order = if state.shuffle {
                state.shuffle_order.clone()
            } else {
                (0..state.queue.len()).collect()
            };
            (state.queue.clone(), order)
        };
        if tracks.is_empty() {
            self.hint(self.text(
                "Open a folder or play a playlist to start a queue.",
                "Abra uma pasta ou toque uma lista de reprodução para iniciar a fila.",
            ));
        }
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some(self.text(
            "Search tracks, artists and albums",
            "Buscar faixas, artistas e álbuns",
        )));
        search.set_text(&self.queue_query.borrow());
        let bar = self.mount_track_search(&search);

        self.scan_metadata(&tracks);
        let manual = self.local_track_sort().is_none();
        let viewport = self.track_viewport(manual, if manual { 2 } else { 0 });
        let items = gtk::StringList::new(&[]);
        let tracks = Rc::new(tracks);
        let order = Rc::new(order);
        let source = tracks.clone();
        let positions: HashMap<_, _> = order
            .iter()
            .enumerate()
            .map(|(position, index)| (*index, position))
            .collect();
        let count = order.len();
        let list = self.virtual_tracks(
            &items,
            move |player, index, display_position| {
                let track = source.get(index)?;
                let (row, title) = player.track_row(track, display_position);
                player.remember_track_row(index, &row);
                if manual {
                    let actions = player.track_actions(&row);
                    let position = *positions.get(&index)?;
                    player.reorder_controls(&row, &actions, position, count, |player, from, to| {
                        if !player.state.borrow_mut().move_queue_track(from, to) {
                            return false;
                        }
                        player.update_position();
                        player.save();
                        player.show(Page::Queue);
                        true
                    });
                }
                Some((row, title))
            },
            |player, index| player.start(index, 0),
        );
        viewport.set_child(Some(&list));
        bar.results.track_list(&list);
        let results = bar.results;
        let weak = Rc::downgrade(self);
        let filter = move |text: &str| {
            let Some(player) = weak.upgrade() else { return };
            replace_indexes(
                &items,
                player.filtered_indexes(&tracks, order.iter().copied(), text),
            );
            results.update(text, items.n_items());
        };
        self.refresh_track_highlight();
        filter(search.text().as_str());
        let weak = Rc::downgrade(self);
        search.connect_search_changed(move |search| {
            if let Some(player) = weak.upgrade() {
                player.queue_query.replace(search.text().to_string());
                filter(search.text().as_str());
            }
        });
    }

    fn playlist(self: &Rc<Self>, index: usize) {
        let (name, tracks) = {
            let state = self.state.borrow();
            let Some(playlist) = state.playlists.get(index) else {
                return;
            };
            (playlist.name.clone(), playlist.tracks.clone())
        };
        self.heading(&name);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let queue = tracks.clone();
        let play = button(self.text("Play", "Tocar"), self, move |player| {
            player.replace_queue_from(queue.clone(), 0, Some(index))
        });
        play.set_sensitive(!tracks.is_empty());
        play.add_css_class("suggested-action");
        actions.append(&play);
        actions.append(&button(
            self.text("Add files", "Adicionar arquivos"),
            self,
            move |player| player.choose_files(Import::Playlist(Some(index))),
        ));
        actions.append(&button(
            self.text("Rename", "Renomear"),
            self,
            move |player| player.ask_name(Some(index), Vec::new()),
        ));
        actions.append(&icon_button(
            "document-save-as-symbolic",
            self.text(
                "Export playlist (M3U8)",
                "Exportar lista de reprodução (M3U8)",
            ),
            self,
            move |player| player.export_playlist(index),
        ));
        actions.append(&self.delete_playlist_button(index));
        let view = self.state.borrow().view;
        if view.playlist_actions {
            self.content.append(&actions);
        }
        let drop = gtk::Label::new(Some(self.text(
            "Drop files or folders here",
            "Solte arquivos ou pastas aqui",
        )));
        drop.set_can_target(true);
        drop.add_css_class("drop-zone");
        self.accept_files(&drop, Import::Playlist(Some(index)));
        if view.playlist_drop_zone {
            self.content.append(&drop);
        }

        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some(self.text(
            "Search tracks, artists and albums",
            "Buscar faixas, artistas e álbuns",
        )));
        {
            let mut query = self.playlist_query.borrow_mut();
            if query.0 != Some(index) {
                *query = (Some(index), String::new());
            }
            search.set_text(&query.1);
        }
        let bar = self.mount_track_search(&search);

        self.scan_metadata(&tracks);
        let manual = self.local_track_sort().is_none();
        let viewport = self.track_viewport(manual, if manual { 3 } else { 1 });
        if !view.playlist_drop_zone {
            self.accept_files(&viewport, Import::Playlist(Some(index)));
        }
        let tracks = Rc::new(tracks);
        let items = gtk::StringList::new(&[]);
        let source = tracks.clone();
        let playback = tracks.clone();
        let list = self.virtual_tracks(
            &items,
            move |player, index_in_playlist, display_position| {
                let track = source.get(index_in_playlist)?;
                let (row, title) = player.track_row(track, display_position);
                player.remember_track_row(index_in_playlist, &row);
                let actions = player.track_actions(&row);
                if manual {
                    player.reorder_controls(
                        &row,
                        &actions,
                        index_in_playlist,
                        source.len(),
                        move |player, from, to| player.move_track(index, from, to),
                    );
                }
                actions.append(&icon_button(
                    "list-remove-symbolic",
                    player.text("Remove from playlist", "Remover da lista de reprodução"),
                    player,
                    move |player| {
                        player.remove_tracks(Page::Playlist(index), vec![index_in_playlist]);
                    },
                ));
                Some((row, title))
            },
            move |player, position| {
                player.replace_queue_from(playback.as_ref().clone(), position, Some(index))
            },
        );
        viewport.set_child(Some(&list));
        bar.results.track_list(&list);
        let results = bar.results;
        let weak = Rc::downgrade(self);
        let filter = move |text: &str| {
            let Some(player) = weak.upgrade() else { return };
            replace_indexes(
                &items,
                player.filtered_indexes(&tracks, 0..tracks.len(), text),
            );
            results.update(text, items.n_items());
        };
        self.refresh_track_highlight();
        filter(search.text().as_str());
        let weak = Rc::downgrade(self);
        search.connect_search_changed(move |search| {
            if let Some(player) = weak.upgrade() {
                player
                    .playlist_query
                    .replace((Some(index), search.text().to_string()));
                filter(search.text().as_str());
            }
        });
    }

    fn remember_track_row(&self, index: usize, row: &gtk::Box) {
        if self.track_highlight.get() == Some(index) {
            row.add_css_class("current");
        }
        let mut rows = self.track_rows.borrow_mut();
        rows.retain(|_, row| row.upgrade().is_some());
        rows.insert(index, row.downgrade());
    }

    fn refresh_track_highlight(&self) {
        let state = self.state.borrow();
        let current = (|| {
            if !self.active.get() {
                return None;
            }
            let track = state.queue.get(state.current)?;
            let playlist = match self.page.get() {
                Page::Queue => return Some(state.current),
                Page::Playlist(index) => state.playlists.get(index)?,
                _ => return None,
            };
            if playlist.tracks == state.queue {
                return Some(state.current);
            }
            playlist.tracks.iter().position(|entry| entry == track)
        })();
        self.track_highlight.set(current);
        for (&position, row) in self.track_rows.borrow().iter() {
            if let Some(row) = row.upgrade() {
                if current == Some(position) {
                    row.add_css_class("current");
                } else {
                    row.remove_css_class("current");
                }
            }
        }
    }

    fn virtual_tracks(
        self: &Rc<Self>,
        strings: &gtk::StringList,
        bind: impl Fn(&Rc<Self>, usize, usize) -> Option<(gtk::Box, gtk::Box)> + 'static,
        activate: impl Fn(&Rc<Self>, usize) + 'static,
    ) -> gtk::ListView {
        let factory = gtk::SignalListItemFactory::new();
        let model = gtk::MultiSelection::new(Some(strings.clone()));
        let row_model = model.clone();
        let activate = Rc::new(activate);
        let row_activate = activate.clone();
        let weak = Rc::downgrade(self);
        let unbind_player = weak.clone();
        let page = self.page.get();
        factory.connect_bind(move |_, item| {
            let (Some(player), Some(item)) = (weak.upgrade(), item.downcast_ref::<gtk::ListItem>())
            else {
                return;
            };
            if player.page.get() != page || player.columns.borrow().is_none() {
                return;
            }
            let Some(index) = item
                .item()
                .and_downcast::<gtk::StringObject>()
                .and_then(|entry| entry.string().parse::<usize>().ok())
            else {
                return;
            };
            if let Some((row, title)) = bind(&player, index, item.position() as usize) {
                player.attach_track_menu(&row, item.position(), &row_model);
                let click = gtk::GestureClick::new();
                click.set_button(1);
                let activate = row_activate.clone();
                let player = Rc::downgrade(&player);
                click.connect_released(move |gesture, count, _, _| {
                    let modifiers = gesture.current_event_state();
                    if count != 1
                        || modifiers.intersects(
                            gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
                        )
                    {
                        return;
                    }
                    let weak = player.clone();
                    let activate = activate.clone();
                    glib::idle_add_local_once(move || {
                        if let Some(player) =
                            weak.upgrade().filter(|player| player.page.get() == page)
                        {
                            activate(&player, index);
                        }
                    });
                });
                title.add_controller(click);
                item.set_child(Some(&row));
            }
        });
        factory.connect_unbind(move |_, item| {
            if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
                if let (Some(player), Some(row)) = (unbind_player.upgrade(), item.child()) {
                    if player
                        .track_popover
                        .upgrade()
                        .and_then(|popover| popover.parent())
                        .as_ref()
                        == Some(&row)
                    {
                        player.close_track_menu();
                    }
                }
                item.set_child(gtk::Widget::NONE);
            }
        });
        let list = gtk::ListView::new(Some(model.clone()), Some(factory));
        list.set_show_separators(false);
        self.track_model.set(Some(&model));
        self.attach_track_keys(&list, &model);
        let weak = Rc::downgrade(self);
        list.connect_activate(move |_, position| {
            if let (Some(player), Some(index)) =
                (weak.upgrade(), track_menu::track_index(&model, position))
            {
                activate(&player, index);
            }
        });
        list.add_css_class("library-tracks");
        list
    }

    fn track_viewport(self: &Rc<Self>, has_grip: bool, actions: usize) -> gtk::ScrolledWindow {
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        header.add_css_class("track-header");
        if has_grip {
            let grip = gtk::Label::new(None);
            grip.set_width_request(30);
            header.append(&grip);
        }
        let reserved = 22 + (actions as i32 * 34) + if has_grip { 36 } else { 0 };
        let columns = TrackColumns::new(reserved);
        let page = self.page.get();
        let state = self.state.borrow();
        let order = state.view.ordered_columns();
        let marker = if page == Page::Library {
            Some((state.sort, state.descending))
        } else {
            self.local_track_sort()
        };
        drop(state);
        for index in order {
            let resize = Rc::downgrade(self);
            let fit = Rc::downgrade(self);
            let reorder = Rc::downgrade(self);
            let label = match marker {
                Some((sort, descending)) => self.column_title(index, true, sort, descending),
                None => self.column_title(index, false, TrackSort::Title, false),
            };
            let weak = Rc::downgrade(self);
            let sort = Some(Box::new(move || {
                if let Some(player) = weak.upgrade() {
                    player.sort_column(index);
                }
            }) as Box<dyn Fn()>);
            header.append(&columns.header(
                index,
                &label,
                self.text(
                    "Drag to resize or double-click to fit",
                    "Arraste para redimensionar ou dê um duplo clique para ajustar ao conteúdo",
                ),
                move |widths, delta, finished| {
                    if let Some(player) = resize.upgrade() {
                        player.resize_column(index, widths, delta, finished);
                    }
                },
                move || {
                    if let Some(player) = fit.upgrade() {
                        player.auto_fit_column(index);
                    }
                },
                move |target| {
                    let Some(player) = reorder.upgrade() else {
                        return false;
                    };
                    if !player.state.borrow_mut().view.move_column(index, target) {
                        return false;
                    }
                    player.save();
                    let weak = Rc::downgrade(&player);
                    glib::idle_add_local_once(move || {
                        if let Some(player) = weak.upgrade() {
                            player.show(player.page.get());
                        }
                    });
                    true
                },
                sort,
            ));
        }
        if actions > 0 {
            let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            columns.align_actions(&spacer);
            header.append(&spacer);
        }
        self.columns.replace(Some(columns));
        self.fit_columns();
        let table = gtk::Box::new(gtk::Orientation::Vertical, 0);
        table.append(&header);
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .overlay_scrolling(true)
            .build();
        table.append(&scroll);
        self.content.append(&table);
        scroll
    }

    fn track_row(self: &Rc<Self>, path: &std::path::Path, position: usize) -> (gtk::Box, gtk::Box) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        style_row(&row, position);
        let metadata = self.info.borrow();
        let info = metadata.get(path);
        let track = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        track.add_css_class("track-title");
        track.set_hexpand(true);
        track.set_cursor_from_name(Some("pointer"));
        let cells = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let columns = self.columns.borrow();
        let columns = columns
            .as_ref()
            .expect("track headers are created before track rows");
        for index in self.state.borrow().view.ordered_columns() {
            cells.append(&columns.cell(index, &track_cell_text(path, info, index)));
        }
        cells.set_hexpand(true);
        track.append(&cells);
        row.append(&track);
        (row, track)
    }

    fn resize_column(
        &self,
        index: usize,
        starting_widths: [Option<i32>; 4],
        delta: i32,
        finished: bool,
    ) {
        let order = self.state.borrow().view.ordered_columns();
        let widths = table::resized_widths(starting_widths, order, index, delta);
        self.apply_column_widths(widths, finished);
    }

    fn auto_fit_column(&self, column: usize) {
        let widths = {
            let columns = self.columns.borrow();
            let (Some(columns), Some(model)) = (columns.as_ref(), self.track_model.upgrade())
            else {
                return;
            };
            let state = self.state.borrow();
            let tracks = match self.page.get() {
                Page::Library => &state.tracks,
                Page::Queue => &state.queue,
                Page::Playlist(index) => match state.playlists.get(index) {
                    Some(playlist) => &playlist.tracks,
                    None => return,
                },
                _ => return,
            };
            let metadata = self.info.borrow();
            let layout = self
                .content
                .create_pango_layout(Some(&columns.title(column)));
            let mut widest = layout.pixel_size().0 + 16;
            for position in 0..model.n_items() {
                if let Some(path) =
                    track_menu::track_index(&model, position).and_then(|index| tracks.get(index))
                {
                    layout.set_text(&track_cell_text(path, metadata.get(path), column));
                    widest = widest.max(layout.pixel_size().0 + 16);
                    if widest >= table::MAXIMUM {
                        break;
                    }
                }
            }
            table::auto_sized_widths(
                columns.allocated_widths(),
                state.view.ordered_columns(),
                column,
                widest,
            )
        };
        self.apply_column_widths(widths, true);
    }

    fn apply_column_widths(&self, widths: [Option<i32>; 4], finished: bool) {
        {
            let mut state = self.state.borrow_mut();
            let view = &mut state.view;
            for (saved, width) in view
                .column_widths
                .iter_mut()
                .zip(widths.into_iter().skip(1))
            {
                if let Some(width) = width {
                    *saved = width;
                }
            }
        }
        self.fit_columns();
        if finished {
            self.save();
        }
    }

    fn track_actions(&self, row: &gtk::Box) -> gtk::Box {
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        self.columns
            .borrow()
            .as_ref()
            .expect("track headers exist")
            .align_actions(&actions);
        row.append(&actions);
        actions
    }

    fn reorder_controls(
        self: &Rc<Self>,
        row: &gtk::Box,
        actions: &gtk::Box,
        position: usize,
        count: usize,
        reorder: impl Fn(&Rc<Self>, usize, usize) -> bool + 'static,
    ) {
        let reorder = Rc::new(reorder);
        let drag = gtk::DragSource::builder()
            .actions(gdk::DragAction::MOVE)
            .build();
        let dragged_position = gtk::StringObject::new(&position.to_string());
        drag.set_content(Some(&gdk::ContentProvider::for_value(
            &dragged_position.to_value(),
        )));
        let handle = gtk::Label::new(Some("⠿"));
        handle.set_width_request(30);
        handle.set_can_target(true);
        handle.set_cursor_from_name(Some("grab"));
        handle.set_tooltip_text(Some(self.text("Drag to reorder", "Arraste para reordenar")));
        handle.add_controller(drag);
        row.prepend(&handle);

        let target = gtk::DropTarget::new(gtk::StringObject::static_type(), gdk::DragAction::MOVE);
        let weak = Rc::downgrade(self);
        let move_by_drag = reorder.clone();
        target.connect_drop(move |_, value, _, _| {
            let (Some(player), Ok(from)) = (weak.upgrade(), value.get::<gtk::StringObject>())
            else {
                return false;
            };
            from.string()
                .parse::<usize>()
                .is_ok_and(|from| move_by_drag(&player, from, position))
        });
        row.add_controller(target);
        let move_up = reorder.clone();
        let up = icon_button(
            "go-up-symbolic",
            self.text("Move up", "Subir"),
            self,
            move |player| {
                move_up(player, position, position.saturating_sub(1));
            },
        );
        up.set_sensitive(position > 0);
        actions.append(&up);
        let down = icon_button(
            "go-down-symbolic",
            self.text("Move down", "Descer"),
            self,
            move |player| {
                reorder(player, position, position + 1);
            },
        );
        down.set_sensitive(position + 1 < count);
        actions.append(&down);
    }

    fn scan_metadata(self: &Rc<Self>, tracks: &[PathBuf]) {
        let known = self.info.borrow();
        let mut reading = self.reading_info.borrow_mut();
        let missing: Vec<_> = tracks
            .iter()
            .filter(|path| !known.contains_key(*path) && reading.insert((*path).clone()))
            .cloned()
            .collect();
        drop(known);
        drop(reading);
        if missing.is_empty() {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || {
                missing
                    .into_iter()
                    .map(|path| {
                        let info = read_track(&path);
                        (path, info)
                    })
                    .collect::<Vec<_>>()
            })
            .await;
            let (Some(player), Ok(items)) = (weak.upgrade(), result) else {
                return;
            };
            {
                let mut known = player.info.borrow_mut();
                let mut reading = player.reading_info.borrow_mut();
                for (path, info) in items {
                    reading.remove(&path);
                    known.insert(path, info);
                }
            }
            let page = player.page.get();
            if page == Page::Library {
                if let Some(refresh) = player.library_refresh.borrow().as_ref() {
                    refresh(&player.library_search.borrow());
                }
            } else if matches!(page, Page::Queue | Page::Playlist(_)) {
                player.show(page);
            }
            player.update_transport();
        });
    }

    fn load_current_track(self: &Rc<Self>) {
        let path = {
            let state = self.state.borrow();
            state.queue.get(state.current).cloned()
        };
        if *self.artwork_track.borrow() == path {
            return;
        }
        self.artwork_track.replace(path.clone());
        self.refresh_lyrics();
        self.artwork_button.set_sensitive(false);
        self.cover.replace(None);
        self.cover_source.set_paintable(gdk::Paintable::NONE);
        self.thumbnail.replace(None);
        self.artwork.set_icon_name(Some("audio-x-generic-symbolic"));
        let Some(path) = path else { return };
        self.scan_metadata(std::slice::from_ref(&path));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let file = path.clone();
            let result = gio::spawn_blocking(move || read_cover_image(&file)).await;
            let (Some(player), Ok(Some((original, thumbnail)))) = (weak.upgrade(), result) else {
                return;
            };
            if player.artwork_track.borrow().as_ref() != Some(&path) {
                return;
            }
            player
                .thumbnail
                .replace(thumbnail.map(|image| image.pixbuf()));
            player.update_thumbnail(player.thumbnail_size.get());
            let cover = gdk::Texture::for_pixbuf(&original.pixbuf());
            player.cover_source.set_paintable(Some(&cover));
            player.cover.replace(Some(cover));
            player.artwork_button.set_sensitive(true);
        });
    }

    fn set_lyrics_enabled(self: &Rc<Self>, enabled: bool) {
        if self.state.borrow().view.lyrics == enabled {
            return;
        }
        self.state.borrow_mut().view.lyrics = enabled;
        if let Some(button) = self.lyrics_preference.upgrade() {
            button.set_active(enabled);
        }
        self.refresh_lyrics();
        self.save();
    }

    fn set_online_lyrics_enabled(self: &Rc<Self>, enabled: bool) {
        if self.state.borrow().view.online_lyrics == enabled {
            return;
        }
        self.state.borrow_mut().view.online_lyrics = enabled;
        if let Some(button) = self.online_lyrics_preference.upgrade() {
            button.set_active(enabled);
        }
        self.refresh_lyrics();
        self.save();
    }

    fn refresh_lyrics(self: &Rc<Self>) {
        use lyrics::Lyrics;
        self.lyric_drag_preview.set(None);
        let request = self.lyric_request.get().wrapping_add(1);
        self.lyric_request.set(request);
        self.lyrics.replace(Lyrics::Missing);
        self.update_lyrics();
        let view = self.state.borrow().view;
        if !view.lyrics {
            return;
        }
        let Some(path) = self.artwork_track.borrow().clone() else {
            return;
        };
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let file = path.clone();
            let local = gio::spawn_blocking(move || Lyrics::read(&file))
                .await
                .unwrap_or_default();
            let Some(player) = weak
                .upgrade()
                .filter(|player| player.lyric_request.get() == request)
            else {
                return;
            };
            if !matches!(local, Lyrics::Missing) || !view.online_lyrics {
                player.lyrics.replace(local);
                player.update_lyrics();
                return;
            }
            if let Some(cached) = player.online_lyrics_cache.borrow().get(&path).cloned() {
                player.lyrics.replace(cached);
                player.update_lyrics();
                return;
            }
            drop(player);
            glib::timeout_future(Duration::from_millis(400)).await;
            if weak
                .upgrade()
                .is_none_or(|player| player.lyric_request.get() != request)
            {
                return;
            }
            let file = path.clone();
            let online = gio::spawn_blocking(move || Lyrics::lookup_online(&read_track(&file)))
                .await
                .ok()
                .flatten()
                .unwrap_or_default();
            let Some(player) = weak
                .upgrade()
                .filter(|player| player.lyric_request.get() == request)
            else {
                return;
            };
            {
                let mut cache = player.online_lyrics_cache.borrow_mut();
                if cache.len() >= 64 {
                    cache.clear();
                }
                cache.insert(path, online.clone());
            }
            player.lyrics.replace(online);
            player.update_lyrics();
        });
    }

    fn update_lyrics(&self) {
        let state = self.state.borrow();
        let lyrics = self.lyrics.borrow();
        let synced = matches!(*lyrics, lyrics::Lyrics::Timed(_));
        let mini = match &*lyrics {
            lyrics::Lyrics::Plain(text) => text.as_str(),
            lyrics::Lyrics::Timed(_) => "",
            lyrics::Lyrics::Missing => self.text(
                "No lyrics available for this track.",
                "Nenhuma letra disponível para esta faixa.",
            ),
        };
        if self.mini_lyrics.text() != mini {
            self.mini_lyrics.set_text(mini);
        }
        set_visible(&self.mini_lyrics, !synced);
        let timing = state
            .queue
            .get(state.current)
            .and_then(|track| state.lyric_offsets_ms.get(track))
            .copied()
            .unwrap_or(0);
        let excerpt = match self.lyric_drag_preview.get() {
            Some(preview) => lyrics.excerpt(preview, 0),
            None => lyrics.excerpt(state.position_ms, timing),
        };
        let has_verse = excerpt[2].is_some_and(|line| !line.is_empty());
        let center = if excerpt[2].is_none() {
            excerpt[3].filter(|line| !line.is_empty()).unwrap_or("♪")
        } else {
            "♪"
        };
        let opacity = if has_verse { 1.0 } else { 0.62 };
        if (self.lyric_lines[2].opacity() - opacity).abs() > 0.01 {
            self.lyric_lines[2].set_opacity(opacity);
        }
        let height = state.view.lyrics_height.clamp(0, 130);
        if self.lyric_strip.height_request() != height {
            self.lyric_strip.set_height_request(height);
        }
        let show_neighbors = self.window.width() >= 760 && matches!(height, 0 | 52..);
        for (index, (widget, text)) in self.lyric_lines.iter().zip(excerpt).enumerate() {
            let text = if synced && !has_verse {
                if index == 2 { center } else { " " }
            } else {
                text.filter(|line| !line.is_empty()).unwrap_or(" ")
            };
            if widget.text() != text {
                widget.set_text(text);
            }
            if matches!(index, 1 | 3) {
                set_visible(widget, show_neighbors);
            }
        }
        set_visible(
            &self.lyric_panel,
            state.view.lyrics && self.active.get() && synced,
        );
    }

    fn show_equalizer(self: &Rc<Self>) {
        if self.equalizer.is_none() {
            gtk::AlertDialog::builder()
                .message(self.text("Equalizer unavailable", "Equalizador indisponível"))
                .detail(self.text(
                    "Install the GStreamer Good Plug-ins to use the equalizer.",
                    "Instale o pacote de plugins Good do GStreamer para usar o equalizador.",
                ))
                .build()
                .show(Some(&self.window));
            return;
        }
        let dialog = gtk::Window::builder()
            .transient_for(&self.window)
            .modal(true)
            .destroy_with_parent(true)
            .default_width(425)
            .title(self.text("Equalizer", "Equalizador"))
            .build();
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 6);
        panel.add_css_class("content");
        let settings = self.state.borrow().equalizer;
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let enabled =
            gtk::CheckButton::with_label(self.text("Enable equalizer", "Ativar equalizador"));
        enabled.set_active(settings.enabled);
        enabled.set_hexpand(true);
        toolbar.append(&enabled);
        let reset = gtk::Button::with_label(self.text("Reset gains", "Zerar ganhos"));
        toolbar.append(&reset);
        panel.append(&toolbar);
        let mut names = vec![self.text("Custom", "Personalizado")];
        names.extend(
            equalizer::PRESETS
                .iter()
                .map(|(en, pt, _)| self.text(en, pt)),
        );
        let preset = gtk::DropDown::from_strings(&names);
        preset.update_property(&[gtk::accessible::Property::Label(
            self.text("Equalizer preset", "Predefinição do equalizador"),
        )]);
        let selection = equalizer::PRESETS
            .iter()
            .position(|(_, _, gains)| *gains == settings.gains);
        preset.set_selected(selection.map_or(0, |index| index as u32 + 1));
        panel.append(&preset);
        let applying = Rc::new(Cell::new(false));
        let mut controls = Vec::new();
        for (index, frequency) in equalizer::BANDS.iter().enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.append(&table_label(frequency, 68));
            let gain = gtk::Scale::with_range(gtk::Orientation::Horizontal, -24.0, 12.0, 1.0);
            gain.set_hexpand(true);
            gain.set_draw_value(true);
            gain.set_digits(0);
            gain.set_value_pos(gtk::PositionType::Right);
            gain.set_format_value_func(|_, value| format!("{value:+.0} dB"));
            gain.add_mark(0.0, gtk::PositionType::Bottom, None);
            gain.set_value(settings.gains[index]);
            gain.update_property(&[gtk::accessible::Property::Label(frequency)]);
            let weak = Rc::downgrade(self);
            let preset = preset.downgrade();
            let applying_preset = applying.clone();
            gain.connect_value_changed(move |slider| {
                if applying_preset.get() {
                    return;
                }
                if let Some(preset) = preset.upgrade() {
                    preset.set_selected(0);
                }
                if let Some(player) = weak.upgrade() {
                    let mut state = player.state.borrow_mut();
                    state.equalizer.gains[index] = slider.value();
                    state
                        .equalizer
                        .apply(player.equalizer.as_ref().expect("checked when opening"));
                }
            });
            row.append(&gain);
            panel.append(&row);
            controls.push(gain);
        }
        let weak = Rc::downgrade(self);
        let preset_enable = enabled.clone();
        preset.connect_selected_notify(move |preset| {
            let Some(index) = preset.selected().checked_sub(1) else {
                return;
            };
            let Some((_, _, gains)) = equalizer::PRESETS.get(index as usize) else {
                return;
            };
            let Some(player) = weak.upgrade() else { return };
            applying.set(true);
            for (control, value) in controls.iter().zip(gains) {
                control.set_value(*value);
            }
            applying.set(false);
            if index > 0 {
                preset_enable.set_active(true);
            }
            let mut state = player.state.borrow_mut();
            state.equalizer.gains = *gains;
            state
                .equalizer
                .apply(player.equalizer.as_ref().expect("checked when opening"));
        });
        let selection = preset.downgrade();
        reset.connect_clicked(move |_| {
            if let Some(preset) = selection.upgrade() {
                preset.set_selected(1);
            }
        });
        let weak = Rc::downgrade(self);
        enabled.connect_toggled(move |enabled| {
            if let Some(player) = weak.upgrade() {
                let mut state = player.state.borrow_mut();
                state.equalizer.enabled = enabled.is_active();
                state
                    .equalizer
                    .apply(player.equalizer.as_ref().expect("checked when opening"));
            }
        });
        let close = gtk::Button::with_label(self.text("Close", "Fechar"));
        let window = dialog.downgrade();
        close.connect_clicked(move |_| {
            if let Some(window) = window.upgrade() {
                window.close();
            }
        });
        panel.append(&close);
        let weak = Rc::downgrade(self);
        dialog.connect_close_request(move |_| {
            if let Some(player) = weak.upgrade() {
                player.save();
            }
            glib::Propagation::Proceed
        });
        dialog.set_child(Some(&panel));
        dialog.present();
    }

    fn show_cover(&self) {
        let cover = self.cover.borrow();
        let Some(cover) = cover.as_ref() else { return };
        let dialog = gtk::Window::builder()
            .transient_for(&self.window)
            .destroy_with_parent(true)
            .default_width(520)
            .default_height(520)
            .resizable(true)
            .title(self.text("Album cover", "Capa do álbum"))
            .build();
        let picture = gtk::Picture::for_paintable(cover);
        picture.set_can_shrink(true);
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_alternative_text(Some(self.text("Album cover", "Capa do álbum")));
        dialog.set_child(Some(&picture));
        dialog.present();
    }

    fn export_playlist(self: &Rc<Self>, index: usize) {
        let playlist = {
            let state = self.state.borrow();
            let source = &state.playlists[index];
            Playlist {
                name: source.name.clone(),
                tracks: source.tracks.clone(),
            }
        };
        let safe_name = playlist.name.replace(['/', '\r', '\n'], "_");
        let title = self.text(
            "Export playlist as M3U8",
            "Exportar lista de reprodução como M3U8",
        );
        let dialog = gtk::FileDialog::builder()
            .title(title)
            .initial_name(format!("{safe_name}.m3u8"))
            .build();
        let weak = Rc::downgrade(self);
        dialog.save(Some(&self.window), gio::Cancellable::NONE, move |result| {
            let (Some(player), Ok(file)) = (weak.upgrade(), result) else {
                return;
            };
            let Some(path) = file.path() else {
                player.message(
                    player.text("Choose a local destination.", "Escolha um destino local."),
                );
                return;
            };
            match playlist.export_m3u8(&path) {
                Ok(()) => player
                    .message(player.text("Playlist exported.", "Lista de reprodução exportada.")),
                Err(error) => {
                    eprintln!("Could not export playlist: {error}");
                    player.message(player.text(
                        "Could not export playlist.",
                        "Não foi possível exportar a lista de reprodução.",
                    ));
                }
            }
        });
    }

    fn show_view_settings(self: &Rc<Self>) {
        let window = gtk::Window::builder()
            .title(self.text("View", "Visualização"))
            .transient_for(&self.window)
            .modal(true)
            .destroy_with_parent(true)
            .default_height(425)
            .build();
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 8);
        panel.add_css_class("content");
        let controls = self.view_controls();
        controls.set_vexpand(true);
        panel.append(&controls);
        let close = gtk::Button::with_label(self.text("Close", "Fechar"));
        close.set_halign(gtk::Align::End);
        let weak = window.downgrade();
        close.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                window.close();
            }
        });
        panel.append(&close);
        window.set_child(Some(&panel));
        window.present();
    }

    fn view_controls(self: &Rc<Self>) -> gtk::Box {
        #[derive(Clone, Copy, PartialEq)]
        enum Scope {
            General,
            Search,
            Column(usize),
            Playlist,
            Lyrics,
            OnlineLyrics,
        }
        type ViewField = fn(&mut ViewOptions) -> &mut bool;
        let choices: [(&str, ViewField, Scope); 19] = [
            (
                self.text("Show sidebar", "Mostrar menu lateral"),
                |view| &mut view.sidebar,
                Scope::General,
            ),
            (
                self.text("Search as a button", "Busca como botão"),
                |view| &mut view.search_button,
                Scope::Search,
            ),
            (
                self.text(
                    "Page title in the top bar",
                    "Título da página na barra superior",
                ),
                |view| &mut view.titlebar_heading,
                Scope::General,
            ),
            (
                self.text("Show title bar icon", "Mostrar ícone na barra de título"),
                |view| &mut view.titlebar_icon,
                Scope::General,
            ),
            (
                self.text("Show artist", "Mostrar artista"),
                |view| &mut view.columns[0],
                Scope::Column(1),
            ),
            (
                self.text("Show album", "Mostrar álbum"),
                |view| &mut view.columns[1],
                Scope::Column(2),
            ),
            (
                self.text("Show track length", "Mostrar duração"),
                |view| &mut view.columns[2],
                Scope::Column(3),
            ),
            (
                self.text("Show library buttons", "Mostrar botões da biblioteca"),
                |view| &mut view.library_actions,
                Scope::General,
            ),
            (
                self.text(
                    "Show playlist buttons",
                    "Mostrar botões da lista de reprodução",
                ),
                |view| &mut view.playlist_actions,
                Scope::Playlist,
            ),
            (
                self.text(
                    "Show playlist drop area",
                    "Mostrar área de soltar da lista de reprodução",
                ),
                |view| &mut view.playlist_drop_zone,
                Scope::Playlist,
            ),
            (
                self.text("Show cover thumbnail", "Mostrar miniatura da capa"),
                |view| &mut view.cover_thumbnail,
                Scope::General,
            ),
            (
                self.text("Show playing track", "Mostrar faixa em reprodução"),
                |view| &mut view.track_info,
                Scope::General,
            ),
            (
                self.text("Show seek bar", "Mostrar barra de progresso"),
                |view| &mut view.seek_bar,
                Scope::General,
            ),
            (
                self.text("Show playback time", "Mostrar tempo de reprodução"),
                |view| &mut view.elapsed,
                Scope::General,
            ),
            (
                self.text("Show shuffle and repeat", "Mostrar aleatório e repetição"),
                |view| &mut view.playback_modes,
                Scope::General,
            ),
            (
                self.text("Show volume", "Mostrar volume"),
                |view| &mut view.volume,
                Scope::General,
            ),
            (
                self.text("Show volume percentage", "Mostrar porcentagem do volume"),
                |view| &mut view.volume_percent,
                Scope::General,
            ),
            (
                self.text("Enable lyrics (Ctrl+Y)", "Ativar letras (Ctrl+Y)"),
                |view| &mut view.lyrics,
                Scope::Lyrics,
            ),
            (
                self.text("Look up lyrics online", "Buscar letras online"),
                |view| &mut view.online_lyrics,
                Scope::OnlineLyrics,
            ),
        ];
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 4);
        panel.add_css_class("view-options");
        let mut current = self.state.borrow().view;
        let mut options = Vec::new();
        for (label, field, scope) in choices {
            let preferred = *field(&mut current);
            let auto_hidden = match scope {
                Scope::Search if self.track_search_field.borrow().is_none() => continue,
                Scope::Playlist
                    if !matches!(self.page.get(), Page::Playlists | Page::Playlist(_)) =>
                {
                    continue;
                }
                Scope::Column(index) => {
                    let columns = self.columns.borrow();
                    let Some(columns) = columns.as_ref() else {
                        continue;
                    };
                    preferred && !columns.column_visible(index)
                }
                _ => false,
            };
            let item = gtk::CheckButton::with_label(label);
            if scope == Scope::OnlineLyrics {
                item.set_tooltip_text(Some(self.text(
                    "Send track tags and duration to LRCLIB when local lyrics are missing",
                    "Envia tags e duração da faixa ao LRCLIB quando não há letra local",
                )));
            }
            item.set_active(preferred && !auto_hidden);
            if auto_hidden {
                item.set_sensitive(false);
                item.set_tooltip_text(Some(self.text(
                    "Widen the window to show this column",
                    "Amplie a janela para mostrar esta coluna",
                )));
            }
            let weak = Rc::downgrade(self);
            item.connect_toggled(move |item| {
                if let Some(player) = weak.upgrade() {
                    match scope {
                        Scope::Lyrics => return player.set_lyrics_enabled(item.is_active()),
                        Scope::OnlineLyrics => {
                            return player.set_online_lyrics_enabled(item.is_active());
                        }
                        _ => {}
                    }
                    *field(&mut player.state.borrow_mut().view) = item.is_active();
                    if scope == Scope::Search {
                        player.track_search_open.set(SearchVisibility::Automatic);
                    }
                    player.apply_view();
                    player.save();
                    player.show(player.page.get());
                }
            });
            panel.append(&item);
            options.push((search::search_key(label), item.downgrade()));
        }
        let empty = gtk::Label::new(Some(
            self.text("No options found", "Nenhuma opção encontrada"),
        ));
        empty.add_css_class("dim-label");
        empty.set_visible(false);
        panel.append(&empty);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some(self.text("Search options", "Buscar opções")));
        let empty = empty.downgrade();
        search.connect_search_changed(move |entry| {
            let terms = search::search_key(&entry.text());
            let mut found = false;
            for (label, option) in &options {
                if let Some(option) = option.upgrade() {
                    let matches = terms.split_whitespace().all(|term| label.contains(term));
                    option.set_visible(matches);
                    found |= matches;
                }
            }
            if let Some(empty) = empty.upgrade() {
                empty.set_visible(!found);
            }
        });
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_height(290)
            .max_content_height(400)
            .propagate_natural_height(true)
            .propagate_natural_width(true)
            .build();
        scroll.set_child(Some(&panel));
        let controls = gtk::Box::new(gtk::Orientation::Vertical, 8);
        controls.append(&search);
        controls.append(&scroll);
        controls
    }

    fn apply_view(&self) {
        let view = self.state.borrow().view;
        self.fit_sidebar();
        set_visible(&self.app_icon, view.titlebar_icon);
        self.artwork_button.set_visible(view.cover_thumbnail);
        self.track_caption.set_visible(view.track_info);
        self.title_grip.set_visible(view.track_info);
        self.fit_transport();
        self.fit_search();
        self.time.set_visible(view.elapsed);
        self.shuffle.set_visible(view.playback_modes);
        self.repeat.set_visible(view.playback_modes);
        self.volume.set_visible(view.volume);
        self.volume.set_draw_value(view.volume_percent);
        self.volume
            .set_width_request(if view.volume_percent { 90 } else { 66 });
        self.fit_columns();
        self.update_lyrics();
    }

    fn fit_transport(&self) {
        let view = self.state.borrow().view;
        let height = view.transport_height.clamp(0, 130);
        let max_width = (self.window.width() / 3).clamp(95, 350);
        let lines = ((height - 9).max(36) / 16).clamp(2, 5) - 1;
        if self.now_playing.lines() != lines {
            self.now_playing.set_lines(lines);
        }
        let thumbnail = (height - 12).clamp(36, if self.window.width() > 760 { 92 } else { 60 });
        if self.thumbnail_size.replace(thumbnail) != thumbnail {
            self.update_thumbnail(thumbnail);
        }
        let width = if view.transport_title_width > 0 {
            view.transport_title_width.clamp(80, max_width)
        } else {
            0
        };
        if self.transport.height_request() != height {
            self.transport.set_size_request(-1, height);
        }
        if self.track_caption.width_request() != width {
            self.track_caption.set_size_request(width, -1);
        }
    }

    fn update_thumbnail(&self, size: i32) {
        self.artwork.set_size_request(size, size);
        self.artwork.set_pixel_size(size);
        let source = self.thumbnail.borrow();
        if let Some(pixbuf) = source.as_ref().and_then(|source| scale_image(source, size)) {
            self.artwork
                .set_paintable(Some(&gdk::Texture::for_pixbuf(&pixbuf)));
        }
    }

    fn seek(&self, seconds: f64) {
        if !self.updating_progress.get() && self.active.get() {
            let target = gst::ClockTime::from_mseconds((seconds.max(0.0) * 1000.0) as u64);
            let sought = self
                .pipeline
                .seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT, target);
            if sought.is_ok() {
                self.pending_seek_signal.set(true);
            }
        }
    }

    fn fit_sidebar(&self) {
        let visible = self.state.borrow().view.sidebar;
        if self.sidebar.is_visible() != visible {
            self.sidebar.set_visible(visible);
        }
    }

    fn fit_columns(&self) {
        let active = self.columns.borrow();
        let Some(columns) = active.as_ref() else {
            return;
        };
        let view = self.state.borrow().view;
        let width = self.content.width();
        // Before the first allocation, use a compact layout so the table does not set a wide window minimum.
        columns.fit(
            if width > 0 { width } else { 400 },
            view.column_widths,
            view.columns,
        );
    }

    fn preferences(self: &Rc<Self>) {
        self.heading(self.text("Preferences", "Preferências"));
        let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let hint = |text: &str| {
            let label = gtk::Label::new(Some(text));
            label.set_xalign(0.0);
            label.set_wrap(true);
            label.add_css_class("muted");
            form.append(&label);
        };
        hint(self.text("Display language", "Idioma da interface"));
        let selector = gtk::DropDown::from_strings(&[
            self.text("Use system language", "Usar idioma do sistema"),
            "English",
            "Português (Brasil)",
            "Español",
            "Français",
        ]);
        selector.add_css_class("preferences-control");
        selector.set_halign(gtk::Align::Start);
        selector.set_selected(match self.state.borrow().language {
            Language::System => 0,
            Language::English => 1,
            Language::Portuguese => 2,
            Language::Spanish => 3,
            Language::French => 4,
        });
        let weak = Rc::downgrade(self);
        selector.connect_selected_notify(move |selector| {
            if let Some(player) = weak.upgrade() {
                player.state.borrow_mut().language = match selector.selected() {
                    1 => Language::English,
                    2 => Language::Portuguese,
                    3 => Language::Spanish,
                    4 => Language::French,
                    _ => Language::System,
                };
                player.save();
                let _ = gtk::prelude::WidgetExt::activate_action(
                    &player.window,
                    "win.refresh-language",
                    None,
                );
            }
        });
        form.append(&selector);

        hint(self.text("Appearance", "Aparência"));
        let theme = gtk::DropDown::from_strings(&[
            self.text("Follow system", "Seguir sistema"),
            self.text("Light", "Claro"),
            self.text("Dark", "Escuro"),
            APP_NAME,
        ]);
        theme.add_css_class("preferences-control");
        theme.set_halign(gtk::Align::Start);
        theme.set_selected(match self.state.borrow().theme {
            Theme::System => 0,
            Theme::Light => 1,
            Theme::Dark => 2,
            Theme::Ferritone => 3,
        });
        let weak = Rc::downgrade(self);
        theme.connect_selected_notify(move |theme| {
            let Some(player) = weak.upgrade() else { return };
            player.state.borrow_mut().theme = match theme.selected() {
                1 => Theme::Light,
                2 => Theme::Dark,
                3 => Theme::Ferritone,
                _ => Theme::System,
            };
            player.apply_theme();
            player.save();
        });
        form.append(&theme);

        hint(self.text("Accent color", "Cor de destaque"));
        let accent = gtk::DropDown::from_strings(&[
            self.text("Follow system", "Seguir sistema"),
            self.text("Green", "Verde"),
            self.text("Blue", "Azul"),
            self.text("Violet", "Violeta"),
            self.text("Copper", "Cobre"),
        ]);
        accent.add_css_class("preferences-control");
        accent.set_halign(gtk::Align::Start);
        accent.set_selected(match self.state.borrow().accent {
            Accent::System => 0,
            Accent::Green => 1,
            Accent::Blue => 2,
            Accent::Violet => 3,
            Accent::Copper => 4,
        });
        let weak = Rc::downgrade(self);
        accent.connect_selected_notify(move |accent| {
            let Some(player) = weak.upgrade() else { return };
            player.state.borrow_mut().accent = match accent.selected() {
                1 => Accent::Green,
                2 => Accent::Blue,
                3 => Accent::Violet,
                4 => Accent::Copper,
                _ => Accent::System,
            };
            player.apply_theme();
            player.save();
        });
        form.append(&accent);

        let menu_icons = gtk::CheckButton::with_label(self.text(
            "Show icons in context menus",
            "Mostrar ícones nos menus de contexto",
        ));
        menu_icons.set_active(self.state.borrow().view.context_menu_icons);
        let weak = Rc::downgrade(self);
        menu_icons.connect_toggled(move |button| {
            if let Some(player) = weak.upgrade() {
                player.state.borrow_mut().view.context_menu_icons = button.is_active();
                player.save();
            }
        });
        form.append(&menu_icons);
        let menu_keycaps = gtk::CheckButton::with_label(self.text(
            "Show keycaps for menu shortcuts",
            "Mostrar teclas nos atalhos dos menus",
        ));
        menu_keycaps.set_active(self.state.borrow().view.context_menu_keycaps);
        let weak = Rc::downgrade(self);
        menu_keycaps.connect_toggled(move |button| {
            if let Some(player) = weak.upgrade() {
                player.state.borrow_mut().view.context_menu_keycaps = button.is_active();
                player.save();
            }
        });
        form.append(&menu_keycaps);

        let heading = gtk::Label::new(Some(self.text("Lyrics", "Letras")));
        heading.add_css_class("section-title");
        heading.set_xalign(0.0);
        form.append(&heading);
        let view = self.state.borrow().view;
        let enabled = gtk::CheckButton::with_label(self.text("Enable lyrics", "Ativar letras"));
        let online = gtk::CheckButton::with_label(
            self.text("Look up lyrics online", "Buscar letras online"),
        );
        enabled.set_active(view.lyrics);
        online.set_active(view.online_lyrics);
        enabled
            .bind_property("active", &online, "sensitive")
            .sync_create()
            .build();
        self.lyrics_preference.set(Some(&enabled));
        self.online_lyrics_preference.set(Some(&online));
        let weak = Rc::downgrade(self);
        enabled.connect_toggled(move |button| {
            if let Some(player) = weak.upgrade() {
                player.set_lyrics_enabled(button.is_active());
            }
        });
        let weak = Rc::downgrade(self);
        online.connect_toggled(move |button| {
            if let Some(player) = weak.upgrade() {
                player.set_online_lyrics_enabled(button.is_active());
            }
        });
        form.append(&enabled);
        form.append(&online);
        hint(self.text(
            "Online lookup sends track tags and duration to LRCLIB.",
            "A busca online envia as tags e a duração da faixa ao LRCLIB.",
        ));
        self.audio_preferences(&form);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&form)
            .build();
        self.content.append(&scroll);
    }

    fn audio_preferences(self: &Rc<Self>, form: &gtk::Box) {
        use replay_gain::Mode;
        let audio = gtk::Label::new(Some(self.text("Audio", "Áudio")));
        audio.add_css_class("section-title");
        audio.set_xalign(0.0);
        form.append(&audio);
        let mode_label = gtk::Label::new(Some("ReplayGain"));
        mode_label.set_xalign(0.0);
        form.append(&mode_label);
        let gain = gtk::DropDown::from_strings(&[
            self.text("Off", "Desativado"),
            self.text("Track gain", "Ganho por faixa"),
            self.text("Album gain", "Ganho por álbum"),
        ]);
        gain.add_css_class("preferences-control");
        gain.set_halign(gtk::Align::Start);
        let saved = self.state.borrow().replay_gain;
        let index = match saved.mode {
            Mode::Off => 0,
            Mode::Track => 1,
            Mode::Album => 2,
        };
        gain.set_selected(index);
        let has_gain = self.replay_gain.is_some();
        gain.set_sensitive(has_gain);
        let protect = gtk::CheckButton::with_label(self.text(
            "Limit gain to avoid clipping",
            "Limitar o ganho para evitar distorção",
        ));
        protect.set_active(saved.protect_peaks);
        protect.set_sensitive(has_gain && index != 0);
        let weak = Rc::downgrade(self);
        let peak_toggle = protect.clone();
        gain.connect_selected_notify(move |gain| {
            let Some(player) = weak.upgrade() else { return };
            player.state.borrow_mut().replay_gain.mode = match gain.selected() {
                1 => Mode::Track,
                2 => Mode::Album,
                _ => Mode::Off,
            };
            peak_toggle.set_sensitive(has_gain && gain.selected() != 0);
            player.save_replay_gain();
        });
        let weak = Rc::downgrade(self);
        protect.connect_toggled(move |protect| {
            if let Some(player) = weak.upgrade() {
                player.state.borrow_mut().replay_gain.protect_peaks = protect.is_active();
                player.save_replay_gain();
            }
        });
        form.append(&gain);
        form.append(&protect);
        let description = gtk::Label::new(Some(if has_gain {
            self.text(
                "ReplayGain levels volume using the audio file's gain tags.",
                "O ReplayGain nivela o volume usando as tags de ganho do arquivo.",
            )
        } else {
            self.text("ReplayGain is unavailable.", "ReplayGain indisponível.")
        }));
        description.set_xalign(0.0);
        description.set_wrap(true);
        form.append(&description);
    }

    fn save_replay_gain(&self) {
        if let Some(replay_gain) = &self.replay_gain {
            replay_gain.apply(self.state.borrow().replay_gain);
        }
        self.save();
    }

    fn apply_theme(&self) {
        let state = self.state.borrow();
        if let Some(settings) = gtk::Settings::default() {
            settings.set_gtk_application_prefer_dark_theme(match state.theme {
                Theme::System => match self
                    .desktop_appearance
                    .as_ref()
                    .map(|appearance| appearance.string("color-scheme"))
                    .as_deref()
                {
                    Some("prefer-dark") => true,
                    Some("prefer-light") => false,
                    _ => self.default_dark,
                },
                Theme::Light => false,
                Theme::Ferritone | Theme::Dark => true,
            });
        }
        let desktop_accent = self
            .desktop_appearance
            .as_ref()
            .filter(|settings| {
                settings
                    .settings_schema()
                    .is_some_and(|schema| schema.has_key("accent-color"))
            })
            .map(|settings| settings.string("accent-color"));
        let accent = state.accent.color(desktop_accent.as_deref());
        let foreground = match accent {
            "@theme_selected_bg_color" => "@theme_selected_fg_color",
            "#9141ac" => "#ffffff",
            _ => "#090b09",
        };
        let (background, text) = if matches!(state.theme, Theme::Ferritone) {
            self.window.add_css_class("ferritone");
            ("#1e1d1b", "#f5f3ef")
        } else {
            self.window.remove_css_class("ferritone");
            ("@theme_bg_color", "@theme_fg_color")
        };
        self.css.load_from_data(
            &include_str!("style.css")
                .replace("PLAYER_BACKGROUND", background)
                .replace("PLAYER_TEXT", text)
                .replace("ACCENT_COLOR", accent)
                .replace("ACCENT_FOREGROUND", foreground),
        );
    }

    fn accept_files(self: &Rc<Self>, widget: &impl IsA<gtk::Widget>, action: Import) {
        let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let weak = Rc::downgrade(self);
        target.connect_drop(move |_, value, _, _| {
            let (Some(player), Ok(files)) = (weak.upgrade(), value.get::<gdk::FileList>()) else {
                return false;
            };
            player.import(
                files.files().iter().filter_map(gio::File::path).collect(),
                action,
            );
            true
        });
        widget.add_controller(target);
    }

    fn choose_folder(self: &Rc<Self>, action: Import) {
        self.choose_folder_from(action, &self.window);
    }

    fn choose_folder_from(self: &Rc<Self>, action: Import, parent: &impl IsA<gtk::Window>) {
        let dialog = gtk::FileDialog::builder()
            .title(self.text("Choose a folder", "Escolha uma pasta"))
            .build();
        if matches!(action, Import::Library) {
            let state = self.state.borrow();
            let location = state.library_roots.first().or_else(|| state.tracks.first());
            if let Some(folder) = location
                .and_then(|path| path.parent())
                .filter(|path| path.is_dir())
            {
                dialog.set_initial_folder(Some(&gio::File::for_path(folder)));
            }
        }
        let weak = Rc::downgrade(self);
        dialog.select_folder(Some(parent), gio::Cancellable::NONE, move |result| {
            if let (Some(player), Ok(file)) = (weak.upgrade(), result) {
                if let Some(path) = file.path() {
                    player.import(vec![path], action);
                } else {
                    player.message(player.text(
                        "Choose a folder mounted or synced to this computer.",
                        "Escolha uma pasta montada ou sincronizada neste computador.",
                    ));
                }
            }
        });
    }

    fn choose_files(self: &Rc<Self>, action: Import) {
        let dialog = gtk::FileDialog::builder()
            .title(if matches!(action, Import::ImportPlaylist) {
                self.text(
                    "Choose playlist files",
                    "Escolha arquivos de listas de reprodução",
                )
            } else {
                self.text("Choose music files", "Escolha arquivos de música")
            })
            .build();
        let weak = Rc::downgrade(self);
        dialog.open_multiple(Some(&self.window), gio::Cancellable::NONE, move |result| {
            if let (Some(player), Ok(files)) = (weak.upgrade(), result) {
                let paths = (0..files.n_items())
                    .filter_map(|i| files.item(i))
                    .filter_map(|item| item.downcast::<gio::File>().ok())
                    .filter_map(|file| file.path())
                    .collect();
                player.import(paths, action);
            }
        });
    }

    fn import(self: &Rc<Self>, paths: Vec<PathBuf>, action: Import) {
        if self.importing.replace(true) {
            return;
        }
        self.message(self.text("Looking for music…", "Procurando músicas…"));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || {
                let roots = if matches!(action, Import::Library) {
                    paths
                        .iter()
                        .filter_map(|path| path.canonicalize().ok())
                        .filter(|path| path.is_dir())
                        .collect()
                } else {
                    Vec::new()
                };
                let accept_playlists = matches!(
                    action,
                    Import::Playlist(_) | Import::PickPlaylist | Import::ImportPlaylist
                );
                let (files, audio): (Vec<_>, Vec<_>) = paths
                    .into_iter()
                    .partition(|path| accept_playlists && playlist_file::is_playlist(path));
                let playlists: Vec<_> = files
                    .iter()
                    .filter_map(|file| playlist_file::read(file).ok())
                    .filter(|playlist| !playlist.tracks.is_empty())
                    .collect();
                let tracks = if matches!(action, Import::ImportPlaylist) {
                    Vec::new()
                } else {
                    collect_tracks(audio)
                };
                (tracks, playlists, roots)
            })
            .await;
            let Some(player) = weak.upgrade() else { return };
            player.importing.set(false);
            let Ok((mut tracks, playlists, roots)) = result else {
                player.message(player.text(
                    "Could not read the selection.",
                    "Não foi possível ler a seleção.",
                ));
                return;
            };
            if matches!(action, Import::Library) {
                let found = !tracks.is_empty();
                {
                    let mut state = player.state.borrow_mut();
                    state.library_roots.extend(roots);
                    state.tracks.extend(tracks);
                    state.tracks.sort();
                    state.tracks.dedup();
                }
                player.refresh_library_visibility();
                if found && player.page.get() != Page::Library {
                    player.show(Page::Library);
                }
                player.message(if found {
                    ""
                } else {
                    player.text(
                        "No supported music or playlists found.",
                        "Nenhuma música ou lista de reprodução compatível encontrada.",
                    )
                });
                return;
            }
            if !playlists.is_empty() {
                if matches!(action, Import::Playlist(None) | Import::ImportPlaylist) {
                    let mut state = player.state.borrow_mut();
                    state.playlists.extend(playlists);
                    drop(state);
                    player.show(Page::Playlists);
                    player.message(
                        player.text("Playlists imported.", "Listas de reprodução importadas."),
                    );
                    player.save();
                    if tracks.is_empty() {
                        return;
                    }
                } else {
                    tracks.extend(playlists.into_iter().flat_map(|playlist| playlist.tracks));
                }
            }
            if tracks.is_empty() {
                player.message(player.text(
                    "No supported music or playlists found.",
                    "Nenhuma música ou lista de reprodução compatível encontrada.",
                ));
                return;
            }
            player.message("");
            match action {
                Import::Play => {
                    player.replace_queue(tracks, 0);
                    player.show(Page::Queue);
                }
                Import::Playlist(Some(index)) => player.add_to_playlist(index, tracks),
                Import::Playlist(None) => player.ask_name(None, tracks),
                Import::PickPlaylist => player.choose_playlist(tracks),
                Import::ImportPlaylist | Import::Library => {}
            }
        });
    }

    fn ask_name(self: &Rc<Self>, index: Option<usize>, tracks: Vec<PathBuf>) {
        let title = if index.is_some() {
            self.text("Rename playlist", "Renomear lista de reprodução")
        } else {
            self.text("New playlist", "Nova lista de reprodução")
        };
        let dialog = gtk::Window::builder()
            .title(title)
            .transient_for(&self.window)
            .modal(true)
            .resizable(false)
            .default_width(380)
            .build();
        let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
        form.add_css_class("content");
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some(
            self.text("Playlist name", "Nome da lista de reprodução"),
        ));
        entry.set_activates_default(true);
        if let Some(index) = index {
            entry.set_text(&self.state.borrow().playlists[index].name);
        }
        form.append(&entry);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.set_halign(gtk::Align::End);
        let cancel = gtk::Button::with_label(self.text("Cancel", "Cancelar"));
        let window = dialog.downgrade();
        cancel.connect_clicked(move |_| {
            if let Some(window) = window.upgrade() {
                window.close();
            }
        });
        actions.append(&cancel);

        let window = dialog.downgrade();
        let name_entry = entry.clone();
        let confirm = button(self.text("Save", "Salvar"), self, move |player| {
            let name = name_entry.text().trim().to_owned();
            if name.is_empty() {
                return;
            }
            let id = {
                let mut state = player.state.borrow_mut();
                if let Some(index) = index {
                    state.playlists[index].name = name;
                    index
                } else {
                    state.playlists.push(Playlist {
                        name,
                        tracks: tracks.clone(),
                    });
                    state.playlists.len() - 1
                }
            };
            if let Some(window) = window.upgrade() {
                window.close();
            }
            player.save();
            player.show(Page::Playlist(id));
        });
        confirm.add_css_class("suggested-action");
        confirm.set_sensitive(!entry.text().trim().is_empty());
        let save_button = confirm.downgrade();
        entry.connect_changed(move |entry| {
            if let Some(button) = save_button.upgrade() {
                button.set_sensitive(!entry.text().trim().is_empty());
            }
        });
        actions.append(&confirm);
        form.append(&actions);
        dialog.set_child(Some(&form));
        dialog.set_default_widget(Some(&confirm));
        dialog.present();
        entry.grab_focus();
    }

    fn add_to_playlist(self: &Rc<Self>, index: usize, tracks: Vec<PathBuf>) {
        self.state.borrow_mut().playlists[index]
            .tracks
            .extend(tracks);
        self.message(self.text(
            "Tracks added to playlist.",
            "Faixas adicionadas à lista de reprodução.",
        ));
        self.save();
        if self.page.get() != Page::Library {
            self.show(Page::Playlist(index));
        }
    }

    fn move_track(self: &Rc<Self>, index: usize, from: usize, to: usize) -> bool {
        let mut state = self.state.borrow_mut();
        let tracks = &mut state.playlists[index].tracks;
        if from == to || from >= tracks.len() || to >= tracks.len() {
            return false;
        }
        let track = tracks.remove(from);
        tracks.insert(to, track);
        drop(state);
        self.save();
        self.show(Page::Playlist(index));
        true
    }

    fn play_library_track(self: &Rc<Self>, track: &std::path::Path) {
        let state = self.state.borrow();
        if state.shuffle {
            let queue: Vec<_> = state
                .tracks
                .iter()
                .filter(|entry| state.library_track_visible(entry))
                .cloned()
                .collect();
            if let Some(index) = queue.iter().position(|entry| entry == track) {
                drop(state);
                self.replace_queue(queue, index);
                return;
            }
        }
        drop(state);
        self.replace_queue(vec![track.to_owned()], 0);
    }

    fn replace_queue(self: &Rc<Self>, tracks: Vec<PathBuf>, index: usize) {
        self.replace_queue_from(tracks, index, None);
    }

    fn replace_queue_from(
        self: &Rc<Self>,
        tracks: Vec<PathBuf>,
        index: usize,
        playlist: Option<usize>,
    ) {
        if tracks.is_empty() {
            return;
        }
        {
            let mut state = self.state.borrow_mut();
            state.queue = tracks;
            state.queue_playlist = playlist;
            state.current = index;
            state.reset_shuffle_order();
        }
        self.start(index, 0);
        if self.page.get() == Page::Queue {
            self.show(Page::Queue);
        }
    }

    fn start(self: &Rc<Self>, index: usize, position_ms: u64) {
        let track = self.state.borrow().queue.get(index).cloned();
        let Some(track) = track else { return };
        self.message("");
        let _ = self.pipeline.set_state(gst::State::Null);
        if let Some(waveform) = &self.waveform {
            waveform.clear();
        }
        self.pipeline
            .set_property("uri", gio::File::for_path(&track).uri().as_str());
        self.pending_seek.set(position_ms);
        self.pending_seek_signal.set(false);
        self.state.borrow_mut().current = index;
        self.state.borrow_mut().position_ms = position_ms;
        let started = self.pipeline.set_state(gst::State::Playing).is_ok();
        self.active.set(started);
        self.playing.set(started);
        if !started {
            self.message(self.text(
                "Could not start playback.",
                "Não foi possível iniciar a reprodução.",
            ));
        }
        self.load_current_track();
        self.update_transport();
        self.refresh_track_highlight();
        self.refresh_playing_playlist();
        self.save();
    }

    fn toggle(self: &Rc<Self>) {
        if self.active.get() {
            let play = !self.playing.get();
            let state = if play {
                gst::State::Playing
            } else {
                gst::State::Paused
            };
            if self.pipeline.set_state(state).is_ok() {
                self.playing.set(play);
                self.update_position();
                self.update_transport();
                self.save();
            }
        } else {
            let (current, position) = {
                let state = self.state.borrow();
                (state.current, state.position_ms)
            };
            self.start(current, position);
        }
    }

    fn step(self: &Rc<Self>, offset: isize) {
        let index = self.state.borrow().next_index(offset, false);
        if let Some(index) = index {
            self.start(index, 0);
        }
    }

    fn update_position(&self) {
        if !self.active.get() || self.pending_seek.get() != 0 {
            return;
        }
        if let Some(position) = self.pipeline.query_position::<gst::ClockTime>() {
            self.state.borrow_mut().position_ms = position.mseconds();
        }
    }

    fn update_transport(&self) {
        let (track, position, has_queue) = {
            let state = self.state.borrow();
            (
                state.queue.get(state.current).cloned(),
                state.position_ms,
                !state.queue.is_empty(),
            )
        };
        let info = self.info.borrow();
        let metadata = track.as_ref().and_then(|path| info.get(path));
        let name = match track.as_ref() {
            Some(track) => metadata
                .and_then(|info| info.title.clone())
                .unwrap_or_else(|| title(track)),
            None => self.text("Nothing playing", "Nenhuma faixa").to_owned(),
        };
        let artist = metadata
            .and_then(|info| info.artist.as_deref())
            .unwrap_or("");
        if self.now_playing.text() != name {
            self.now_playing.set_text(&name);
            self.now_playing.set_tooltip_text(Some(&name));
        }
        if self.playing_artist.text() != artist {
            self.playing_artist.set_text(artist);
            self.playing_artist
                .set_tooltip_text((!artist.is_empty()).then_some(artist));
        }
        set_visible(&self.playing_artist, !artist.is_empty());
        if self.toggle.is_sensitive() != has_queue {
            self.toggle.set_sensitive(has_queue);
        }
        let (icon, hint) = if self.playing.get() {
            (
                "media-playback-pause-symbolic",
                self.text("Pause", "Pausar"),
            )
        } else {
            ("media-playback-start-symbolic", self.text("Play", "Tocar"))
        };
        if self.toggle.icon_name().as_deref() != Some(icon) {
            self.toggle.set_icon_name(icon);
        }
        if self.toggle.tooltip_text().as_deref() != Some(hint) {
            self.toggle.set_tooltip_text(Some(hint));
        }
        let duration = self
            .pipeline
            .query_duration::<gst::ClockTime>()
            .map_or(0, |value| value.mseconds());
        self.updating_progress.set(true);
        let upper = (duration as f64 / 1000.0).max(1.0);
        if self.progress.adjustment().upper() != upper {
            self.progress.set_range(0.0, upper);
        }
        if self.progress.get_sensitive() != (duration > 0) {
            self.progress.set_sensitive(duration > 0);
        }
        let value = if duration > 0 {
            position as f64 / 1000.0
        } else {
            0.0
        };
        if self.progress.value() != value {
            self.progress.set_value(value);
        }
        self.updating_progress.set(false);
        let display_duration = if duration > 0 {
            duration
        } else {
            metadata.and_then(|info| info.duration_ms).unwrap_or(0)
        };
        let total = timestamp(display_duration);
        let width = (timestamp(position.max(display_duration)).len() + 3 + total.len()) as i32;
        if self.time.width_chars() != width {
            self.time.set_width_chars(width);
        }
        let time = format!("{} / {total}", timestamp(position));
        if self.time.text() != time {
            self.time.set_text(&time);
        }
        self.update_lyrics();
        if let Some(session) = self.media_session.borrow().as_ref() {
            session.publish(self);
        }
    }

    fn update_repeat(&self) {
        let (icon, hint, enabled) = match self.state.borrow().repeat {
            Repeat::Off => (
                "media-playlist-repeat-symbolic",
                self.text("Repeat off", "Repetição desativada"),
                false,
            ),
            Repeat::Queue => (
                "media-playlist-repeat-symbolic",
                self.text("Repeat queue", "Repetir fila"),
                true,
            ),
            Repeat::Track => (
                "media-playlist-repeat-song-symbolic",
                self.text("Repeat track", "Repetir faixa"),
                true,
            ),
        };
        self.repeat.set_icon_name(icon);
        self.repeat.set_tooltip_text(Some(hint));
        self.repeat
            .update_property(&[gtk::accessible::Property::Label(hint)]);
        if enabled {
            self.repeat.add_css_class("enabled");
        } else {
            self.repeat.remove_css_class("enabled");
        }
    }

    fn poll(self: &Rc<Self>) {
        if let Some(bus) = self.pipeline.bus() {
            while let Some(message) = bus.pop() {
                match message.view() {
                    gst::MessageView::Eos(..) => {
                        let index = self.state.borrow().next_index(1, true);
                        if let Some(index) = index {
                            self.start(index, 0);
                        } else {
                            self.stop();
                        }
                    }
                    gst::MessageView::Error(error) => {
                        eprintln!("Playback error: {}", error.error());
                        self.stop();
                        self.message(self.text(
                            "This track could not be played.",
                            "Não foi possível tocar esta faixa.",
                        ));
                    }
                    gst::MessageView::AsyncDone(..) => {
                        let position = self.pending_seek.replace(0);
                        if position > 0 {
                            self.seek(position as f64 / 1000.0);
                        } else if self.pending_seek_signal.replace(false) {
                            if let (Some(session), Some(position)) = (
                                self.media_session.borrow().as_ref(),
                                self.pipeline.query_position::<gst::ClockTime>(),
                            ) {
                                session.seeked(position.useconds() as i64);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        self.update_position();
        self.update_transport();
        if self.window.is_mapped() {
            self.fit_transport();
            self.fit_search();
            self.fit_columns();
        }
    }

    fn stop(&self) {
        let _ = self.pipeline.set_state(gst::State::Null);
        self.active.set(false);
        self.playing.set(false);
        if let Some(waveform) = &self.waveform {
            waveform.clear();
        }
        self.pending_seek.set(0);
        self.pending_seek_signal.set(false);
        self.state.borrow_mut().position_ms = 0;
        self.refresh_track_highlight();
        self.refresh_playing_playlist();
        self.save();
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
