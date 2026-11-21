//! The player's state and the controller methods every page uses: messages, saving, and
//! switching pages. Pure helpers for sorting and matching tracks live here too.

use super::*;

pub(crate) type LibraryRefresh = Rc<dyn Fn(&str)>;
pub(crate) type PlaylistIndicator = (usize, glib::WeakRef<gtk::Box>, glib::WeakRef<gtk::Image>);

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Page {
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
}

#[derive(Clone, Copy)]
pub(crate) enum Import {
    Play,
    Library,
    Playlist(Option<usize>),
    PickPlaylist,
    ImportPlaylist,
}

pub(crate) struct Player {
    pub(crate) window: adw::ApplicationWindow,
    pub(crate) app_icon: gtk::Image,
    pub(crate) content: gtk::Box,
    pub(crate) sidebar: gtk::Box,
    /// Holds the sidebar beside the pages, or over them in a narrow window.
    pub(crate) split_view: adw::OverlaySplitView,
    /// The sidebar's page buttons, to mark the page on screen.
    pub(crate) sidebar_items: RefCell<Vec<(Page, gtk::Button)>>,
    pub(crate) navigation: gtk::MenuButton,
    pub(crate) columns: RefCell<Option<TrackColumns>>,
    pub(crate) track_rows: RefCell<HashMap<usize, glib::WeakRef<gtk::Box>>>,
    pub(crate) track_highlight: Cell<Option<usize>>,
    pub(crate) playlist_indicators: RefCell<Vec<PlaylistIndicator>>,
    pub(crate) playlist_query: RefCell<(Option<usize>, String)>,
    pub(crate) saved_playlist_query: RefCell<String>,
    pub(crate) playlist_menu_delete: gtk::Button,
    /// Shows short messages over the page.
    pub(crate) toasts: adw::ToastOverlay,
    /// The message on screen, replaced by the next one.
    pub(crate) toast: RefCell<Option<adw::Toast>>,
    /// The Preferences dialog while it is open.
    pub(crate) preferences_dialog: glib::WeakRef<adw::PreferencesDialog>,
    pub(crate) now_playing: gtk::Label,
    pub(crate) playing_artist: gtk::Label,
    pub(crate) track_caption: gtk::Box,
    pub(crate) lyric_panel: gtk::Box,
    pub(crate) lyric_strip: gtk::CenterBox,
    pub(crate) lyric_lines: [gtk::Label; 5],
    pub(crate) mini_lyrics: gtk::Label,
    pub(crate) lyrics_preference: glib::WeakRef<adw::SwitchRow>,
    pub(crate) online_lyrics_preference: glib::WeakRef<adw::SwitchRow>,
    pub(crate) lyrics: RefCell<lyrics::Lyrics>,
    pub(crate) lyric_request: Cell<u64>,
    pub(crate) lyric_drag_preview: Cell<Option<u64>>,
    pub(crate) online_lyrics_cache: RefCell<HashMap<PathBuf, lyrics::Lyrics>>,
    pub(crate) artwork: gtk::Image,
    pub(crate) thumbnail: RefCell<Option<gtk::gdk_pixbuf::Pixbuf>>,
    pub(crate) thumbnail_size: Cell<i32>,
    pub(crate) compact_search: gtk::Button,
    pub(crate) track_search_field: RefCell<Option<SearchBar>>,
    pub(crate) track_search_open: Cell<SearchVisibility>,
    pub(crate) track_model: glib::WeakRef<gtk::MultiSelection>,
    pub(crate) track_popover: glib::WeakRef<gtk::Popover>,
    pub(crate) library_refresh: RefCell<Option<LibraryRefresh>>,
    pub(crate) library_manager_rows: glib::WeakRef<gtk::Box>,
    pub(crate) local_sort: Cell<Option<(Page, TrackSort, bool)>>,
    pub(crate) queue_query: RefCell<String>,
    pub(crate) artwork_button: gtk::Button,
    pub(crate) cover_source: gtk::Picture,
    pub(crate) mini: RefCell<Option<gtk::ApplicationWindow>>,
    pub(crate) media_session: RefCell<Option<mpris::MediaSession>>,
    pub(crate) waveform: Option<waveform::Waveform>,
    pub(crate) transport: gtk::Box,
    pub(crate) title_grip: gtk::Box,
    pub(crate) volume: gtk::Scale,
    pub(crate) equalizer: Option<gst::Element>,
    pub(crate) replay_gain: Option<replay_gain::ReplayGain>,
    pub(crate) cover: RefCell<Option<gdk::Texture>>,
    pub(crate) artwork_track: RefCell<Option<PathBuf>>,
    pub(crate) info: RefCell<HashMap<PathBuf, TrackInfo>>,
    pub(crate) reading_info: RefCell<HashSet<PathBuf>>,
    pub(crate) library_search: RefCell<String>,
    pub(crate) shuffle: gtk::ToggleButton,
    pub(crate) repeat: gtk::Button,
    /// The theme's colors; empty for GNOME/Adwaita.
    pub(crate) css: gtk::CssProvider,
    /// Sizes and spacing, loaded for every theme.
    pub(crate) layout_css: gtk::CssProvider,
    pub(crate) desktop_appearance: Option<gio::Settings>,
    /// The desktop's color scheme and accent as the settings portal last reported them.
    pub(crate) portal_color_scheme: Cell<Option<u32>>,
    pub(crate) portal_accent: Cell<Option<&'static str>>,
    /// Keeps the portal signal subscription alive.
    pub(crate) desktop_connection: RefCell<Option<gio::DBusConnection>>,
    pub(crate) toggle: gtk::Button,
    pub(crate) progress: gtk::Scale,
    pub(crate) time: gtk::Label,
    pub(crate) pipeline: gst::Element,
    pub(crate) state: RefCell<Library>,
    pub(crate) storage: Option<PathBuf>,
    pub(crate) page: Cell<Page>,
    pub(crate) active: Cell<bool>,
    pub(crate) playing: Cell<bool>,
    pub(crate) updating_progress: Cell<bool>,
    pub(crate) pending_seek: Cell<u64>,
    pub(crate) pending_seek_signal: Cell<bool>,
    pub(crate) importing: Cell<bool>,
    /// Consecutive tracks that failed to play.
    pub(crate) failures: RefCell<playback::Failures>,
    /// When the playback position is next due to be saved while playing.
    pub(crate) autosave: RefCell<playback::Autosave>,
    /// The next track, handed to GStreamer before the current one ends.
    pub(crate) handoff: Arc<Mutex<playback::Handoff>>,
    /// The phone remote's server while it runs.
    pub(crate) remote: RefCell<Option<remote::Server>>,
    pub(crate) remote_shared: Arc<remote::Shared>,
    pub(crate) remote_sender: mpsc::Sender<remote::Command>,
    pub(crate) remote_receiver: mpsc::Receiver<remote::Command>,
    pub(crate) remote_ticks: Cell<u32>,
    /// What the library list for the phone was built from.
    pub(crate) remote_library_signature: RefCell<Option<(usize, Option<PathBuf>, usize)>>,
    pub(crate) sleep: Cell<playback::SleepTimer>,
    /// The session inhibition that keeps the computer awake while playing; 0 when none.
    pub(crate) inhibit_cookie: Cell<u32>,
    /// Whether the current track's play has been counted.
    pub(crate) play_counted: Cell<bool>,
    /// When the current track started, for ListenBrainz.
    pub(crate) track_started_at: Cell<u64>,
    pub(crate) listens_sending: Cell<bool>,
    /// Whether the ListenBrainz token works, and for which user.
    pub(crate) listenbrainz_state: RefCell<scrobble::Connection>,
    /// Listens ListenBrainz accepted since Ferritone started.
    pub(crate) listens_sent: Cell<usize>,
    pub(crate) listenbrainz_status: RefCell<Option<glib::WeakRef<adw::ActionRow>>>,
    /// Watches the library folders for added and removed files.
    pub(crate) library_monitors: RefCell<Vec<gio::FileMonitor>>,
    pub(crate) selected_album: RefCell<albums::AlbumKey>,
    pub(crate) selected_artist: RefCell<String>,
    /// Album cover thumbnails by the album's first track; `None` when it has no cover.
    pub(crate) album_covers: RefCell<HashMap<PathBuf, Option<gdk::Texture>>>,
    pub(crate) library_rescan_timer: RefCell<Option<glib::SourceId>>,
    pub(crate) library_scanning: Cell<bool>,
    pub(crate) library_rescan_again: Cell<bool>,
    /// The volume last given to the pipeline, after any sleep fade.
    pub(crate) applied_volume: Cell<f64>,
}

impl Player {
    pub(crate) fn text<'a>(&self, english: &'a str, portuguese: &'a str) -> &'a str {
        self.state
            .borrow()
            .language
            .resolve()
            .text(english, portuguese)
    }

    /// Shows a message over the page. It can be closed and goes away on its own after a while.
    pub(crate) fn message(&self, message: &str) {
        if let Some(previous) = self.toast.take() {
            previous.dismiss();
        }
        if message.is_empty() {
            return;
        }
        // Long messages stay a little longer so there is time to read them.
        let seconds = 6 + message.chars().count() as u32 / 25;
        let toast = adw::Toast::builder()
            .title(message)
            .use_markup(false)
            .timeout(seconds)
            .build();
        self.toasts.add_toast(toast.clone());
        self.toast.replace(Some(toast));
    }

    pub(crate) fn save(&self) {
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

    pub(crate) fn save_position(&self) {
        self.autosave.borrow_mut().saved(Instant::now());
        if let Some(path) = &self.storage {
            if let Err(error) = self.state.borrow().save_position(path) {
                eprintln!("Could not save the playback position: {error}");
            }
        }
    }

    pub(crate) fn show(self: &Rc<Self>, page: Page) {
        self.close_track_menu();
        let previous = self.page.replace(page);
        // Over a narrow window the sidebar steps aside once a page is chosen.
        if self.split_view.is_collapsed() {
            self.split_view.set_show_sidebar(false);
        }
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
                Page::Albums | Page::Album | Page::Artists | Page::Artist => {}
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
        }
        self.fit_search();
    }

    pub(crate) fn heading(&self, title: &str) {
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

    pub(crate) fn hint(&self, text: &str) {
        let label = gtk::Label::new(Some(text));
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.add_css_class("muted");
        self.content.append(&label);
    }
}

pub(crate) fn sort_tracks(
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

pub(crate) fn track_matches(path: &std::path::Path, info: Option<&TrackInfo>, query: &str) -> bool {
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

pub(crate) fn library_collation_key(value: &str) -> glib::FilenameCollationKey {
    let normalized = value.trim_start_matches(|character: char| !character.is_alphanumeric());
    glib::FilenameCollationKey::from(if normalized.is_empty() {
        value
    } else {
        normalized
    })
}

pub(crate) fn timestamp(milliseconds: u64) -> String {
    let seconds = milliseconds / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

pub(crate) fn track_cell_text<'a>(
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
