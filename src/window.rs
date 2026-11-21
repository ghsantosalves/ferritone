//! Builds the main window, lays out its parts, and connects their signals.

use super::*;

pub(crate) fn build(app: &adw::Application) {
    if let Some(window) = app.active_window() {
        window.present();
        return;
    }
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(APP_NAME)
        .icon_name(APP_ID)
        .default_width(1040)
        .default_height(670)
        .build();
    let Some((pipeline, handoff)) = playback_pipeline(&window) else {
        return;
    };
    let path = data_path();
    let (mut state, storage, load_error) = match Library::load(&path) {
        Ok(value) => (value, Some(path), None),
        Err(error) => (Library::default(), None, Some(error)),
    };

    state.ensure_shuffle_order();
    let (player, titlebar, sidebar_toggle) =
        create_player(window, pipeline, handoff, state, storage);
    let (previous, next, resize_strip) = lay_out(&player, &titlebar, &sidebar_toggle);
    connect_navigation(&player, &previous, &next, &resize_strip);
    install_window_actions(app, &player);
    connect_playback_controls(&player, &previous, &next);
    start_timers(&player);
    start(&player, load_error);
}

/// The playbin, or `None` after telling the user which plugins are missing.
fn playback_pipeline(
    window: &adw::ApplicationWindow,
) -> Option<(gst::Element, Arc<Mutex<playback::Handoff>>)> {
    let Ok(pipeline) = gst::ElementFactory::make("playbin").build() else {
        let dialog = gtk::AlertDialog::builder()
            .modal(true)
            .message(Language::System.resolve().text(
                "Install the GStreamer base plugins to start playback.",
                "Instale os plugins básicos do GStreamer para reproduzir áudio.",
            ))
            .build();
        dialog.show(Some(window));
        window.present();
        return None;
    };
    // Keep video and subtitle windows out of the audio player.
    pipeline.set_property_from_str("flags", "audio");
    pipeline.set_property("volume", 0.7f64);
    // Queue the next file before the current one ends so tracks follow each other without a gap.
    let handoff = Arc::new(Mutex::new(playback::Handoff::default()));
    let shared = handoff.clone();
    pipeline.connect("about-to-finish", false, move |values| {
        let next = shared.lock().ok().and_then(|mut handoff| handoff.take());
        if let (Some(track), Ok(playbin)) = (next, values[0].get::<gst::Element>()) {
            playbin.set_property("uri", gio::File::for_path(&track).uri().as_str());
        }
        None
    });
    Some((pipeline, handoff))
}

/// Creates the player's widgets and state; the caller lays them out.
fn create_player(
    window: adw::ApplicationWindow,
    pipeline: gst::Element,
    handoff: Arc<Mutex<playback::Handoff>>,
    state: Library,
    storage: Option<PathBuf>,
) -> (Rc<Player>, adw::HeaderBar, gtk::ToggleButton) {
    let (remote_sender, remote_receiver) = mpsc::channel();
    let (equalizer, waveform, replay_gain) = waveform::attach(&pipeline);
    if let Some(equalizer) = &equalizer {
        state.equalizer.apply(equalizer);
    }
    if let Some(replay_gain) = &replay_gain {
        replay_gain.apply(state.replay_gain);
    }
    let titlebar = adw::HeaderBar::new();
    titlebar.add_css_class("compact-titlebar");
    let app_icon = gtk::Image::from_icon_name(APP_ID);
    app_icon.set_pixel_size(27);
    titlebar.pack_start(&app_icon);
    let sidebar_toggle = gtk::ToggleButton::builder()
        .icon_name("sidebar-show-symbolic")
        .build();
    titlebar.pack_start(&sidebar_toggle);
    let navigation = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .build();
    navigation.add_css_class("transport-menu");

    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.set_hexpand(true);
    content.set_vexpand(true);
    content.add_css_class("content");

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
    let split_view = adw::OverlaySplitView::builder()
        .min_sidebar_width(172.0)
        .max_sidebar_width(260.0)
        .sidebar_width_fraction(0.0)
        .build();
    let shuffle = gtk::ToggleButton::builder()
        .icon_name("media-playlist-shuffle-symbolic")
        .active(state.shuffle)
        .build();
    let repeat = gtk::Button::from_icon_name("media-playlist-repeat-symbolic");
    let css = gtk::CssProvider::new();
    let layout_css = gtk::CssProvider::new();
    layout_css.load_from_data(include_str!("layout.css"));
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
        split_view,
        sidebar_items: RefCell::default(),
        navigation,
        columns: RefCell::new(None),
        track_rows: RefCell::new(HashMap::new()),
        track_highlight: Cell::new(None),
        playlist_indicators: RefCell::new(Vec::new()),
        playlist_query: RefCell::new((None, String::new())),
        saved_playlist_query: RefCell::new(String::new()),
        playlist_menu_delete: gtk::Button::new(),
        toasts: adw::ToastOverlay::new(),
        toast: RefCell::new(None),
        preferences_dialog: glib::WeakRef::new(),
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
        layout_css,
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

    (player, titlebar, sidebar_toggle)
}

/// Places the sidebar, pages, header and playback bar in the window and applies the theme.
fn lay_out(
    player: &Rc<Player>,
    titlebar: &adw::HeaderBar,
    sidebar_toggle: &gtk::ToggleButton,
) -> (gtk::Button, gtk::Button, gtk::Box) {
    player.split_view.set_sidebar(Some(&player.sidebar));
    player.split_view.set_content(Some(&player.content));
    player.split_view.set_vexpand(true);
    player.toasts.set_child(Some(&player.split_view));
    // In a narrow window the sidebar folds away and opens over the page from the header.
    sidebar_toggle.set_tooltip_text(Some(player.text("Show sidebar", "Mostrar barra lateral")));
    player
        .split_view
        .bind_property("collapsed", sidebar_toggle, "visible")
        .sync_create()
        .build();
    player
        .split_view
        .bind_property("show-sidebar", sidebar_toggle, "active")
        .sync_create()
        .bidirectional()
        .build();
    let weak = Rc::downgrade(&player);
    player.split_view.connect_collapsed_notify(move |_| {
        if let Some(player) = weak.upgrade() {
            player.fit_sidebar();
        }
    });
    let narrow = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        600.0,
        adw::LengthUnit::Sp,
    ));
    narrow.add_setter(&player.split_view, "collapsed", Some(&true.to_value()));
    player.window.add_breakpoint(narrow);

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
    let playback_bar = gtk::Box::new(gtk::Orientation::Vertical, 0);
    playback_bar.append(&player.lyric_panel);
    playback_bar.append(&resize_strip);
    playback_bar.append(&player.transport);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(titlebar);
    toolbar.set_content(Some(&player.toasts));
    toolbar.add_bottom_bar(&playback_bar);
    player.window.set_content(Some(&toolbar));
    if let Some(display) = gdk::Display::default() {
        // The theme's colors rank above the layout so they replace its plain fallbacks.
        gtk::style_context_add_provider_for_display(
            &display,
            &player.layout_css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION - 1,
        );
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

    (previous, next, resize_strip)
}

/// Fills the sidebar and the navigation menu, and refills them when the language changes.
fn connect_navigation(
    player: &Rc<Player>,
    previous: &gtk::Button,
    next: &gtk::Button,
    resize_strip: &gtk::Box,
) {
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
        let (previous, next, resize_strip) = (previous.clone(), next.clone(), resize_strip.clone());
        move || {
            let Some(player) = player.upgrade() else {
                return;
            };
            clear(&player.sidebar);
            let navigation_popup = gtk::Popover::new();
            player
                .navigation
                .set_tooltip_text(Some(player.text("Navigation", "Navegação")));
            navigation_popup.add_css_class("player-menu");
            let sections = gtk::Box::new(gtk::Orientation::Vertical, 2);
            // With the sidebar hidden, the pages show as a compact grid of icons at the top.
            let navigation_items = gtk::Grid::builder()
                .column_spacing(4)
                .row_spacing(4)
                .column_homogeneous(true)
                .build();
            navigation_items.add_css_class("nav-tiles");
            let navigation_separator = gtk::Separator::new(gtk::Orientation::Horizontal);
            for widget in [
                navigation_items.upcast_ref::<gtk::Widget>(),
                navigation_separator.upcast_ref(),
            ] {
                player
                    .split_view
                    .bind_property("show-sidebar", widget, "visible")
                    .invert_boolean()
                    .sync_create()
                    .build();
            }
            sections.append(&navigation_items);
            sections.append(&navigation_separator);
            let wordmark = gtk::Label::new(Some(APP_NAME));
            wordmark.add_css_class("wordmark");
            player.sidebar.append(&wordmark);
            player.sidebar_items.borrow_mut().clear();
            // `None` opens Preferences, a dialog rather than a page.
            for (index, (page, en, pt, icon)) in [
                (Some(Page::Home), "Home", "Início", "go-home-symbolic"),
                (
                    Some(Page::Library),
                    "Library",
                    "Biblioteca",
                    "folder-music-symbolic",
                ),
                (
                    Some(Page::Albums),
                    "Albums",
                    "Álbuns",
                    "media-optical-symbolic",
                ),
                (
                    Some(Page::Artists),
                    "Artists",
                    "Artistas",
                    "avatar-default-symbolic",
                ),
                (
                    Some(Page::Playlists),
                    "Playlists",
                    "Listas de reprodução",
                    "view-list-symbolic",
                ),
                (
                    Some(Page::Queue),
                    "Queue",
                    "Fila",
                    "media-playlist-consecutive-symbolic",
                ),
                (
                    None,
                    "Preferences",
                    "Preferências",
                    "preferences-system-symbolic",
                ),
            ]
            .into_iter()
            .enumerate()
            {
                let open = move |player: &Rc<Player>| match page {
                    Some(page) => player.show(page),
                    None => player.show_preferences(),
                };
                let item = button("", &player, open);
                let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                line.append(&gtk::Image::from_icon_name(icon));
                let label = gtk::Label::new(Some(player.text(en, pt)));
                label.set_xalign(0.0);
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                line.append(&label);
                item.set_child(Some(&line));
                item.set_tooltip_text(Some(player.text(en, pt)));
                if let Some(page) = page {
                    player.sidebar_items.borrow_mut().push((page, item.clone()));
                }
                if page == Some(Page::Playlists) {
                    player.accept_files(&item, Import::PickPlaylist);
                }
                player.sidebar.append(&item);
                let popup = navigation_popup.downgrade();
                let tile = button("", &player, move |player| {
                    if let Some(popup) = popup.upgrade() {
                        popup.popdown();
                    }
                    open(player);
                });
                tile.add_css_class("nav-tile");
                let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
                content.append(&gtk::Image::from_icon_name(icon));
                let short = if page == Some(Page::Playlists) {
                    player.text("Playlists", "Listas")
                } else {
                    player.text(en, pt)
                };
                let caption = gtk::Label::new(Some(short));
                caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
                content.append(&caption);
                tile.set_child(Some(&content));
                tile.set_tooltip_text(Some(player.text(en, pt)));
                navigation_items.attach(&tile, index as i32 % 4, index as i32 / 4, 1, 1);
            }
            let icons = player.state.borrow().view.context_menu_icons;
            let item =
                |label: &str, accelerator: &str, icon: &str, action: Box<dyn Fn(&Rc<Player>)>| {
                    let popup = navigation_popup.downgrade();
                    shortcut_button(
                        label,
                        accelerator,
                        icons.then_some(icon),
                        false,
                        &player,
                        move |player| {
                            if let Some(popup) = popup.upgrade() {
                                popup.popdown();
                            }
                            action(player);
                        },
                    )
                };
            let window_action = |name: &'static str| -> Box<dyn Fn(&Rc<Player>)> {
                Box::new(move |player: &Rc<Player>| {
                    let _ = gtk::prelude::WidgetExt::activate_action(&player.window, name, None);
                })
            };
            let groups: [Vec<gtk::Button>; 3] = [
                vec![
                    item(
                        player.text("Open folder", "Abrir pasta"),
                        "<Primary>k",
                        "folder-open-symbolic",
                        window_action("win.open-folder"),
                    ),
                    item(
                        player.text("Open files", "Abrir arquivos"),
                        "<Primary>o",
                        "document-open-symbolic",
                        window_action("win.open-files"),
                    ),
                    item(
                        player.text("Import playlist", "Importar lista"),
                        "<Primary><Shift>o",
                        "list-add-symbolic",
                        window_action("win.import-playlist"),
                    ),
                ],
                vec![
                    item(
                        player.text("Mini player", "Miniplayer"),
                        "<Primary>m",
                        "window-new-symbolic",
                        Box::new(|player: &Rc<Player>| mini::open(player)),
                    ),
                    item(
                        player.text("Equalizer", "Equalizador"),
                        "<Primary>e",
                        "audio-speakers-symbolic",
                        Box::new(|player: &Rc<Player>| player.show_equalizer()),
                    ),
                    item(
                        player.text("Sleep timer", "Timer para desligar"),
                        "",
                        "alarm-symbolic",
                        Box::new(|player: &Rc<Player>| player.show_sleep_timer()),
                    ),
                    item(
                        player.text("Remote control", "Controle remoto"),
                        "",
                        "phone-symbolic",
                        Box::new(|player: &Rc<Player>| player.show_remote()),
                    ),
                ],
                vec![
                    item(
                        player.text("View", "Visualização"),
                        "",
                        "view-grid-symbolic",
                        Box::new(|player: &Rc<Player>| {
                            let weak = Rc::downgrade(player);
                            glib::idle_add_local_once(move || {
                                if let Some(player) = weak.upgrade() {
                                    player.show_view_settings();
                                }
                            });
                        }),
                    ),
                    item(
                        player.text("Manage library", "Gerenciar biblioteca"),
                        "<Primary><Shift>b",
                        "document-edit-symbolic",
                        Box::new(|player: &Rc<Player>| player.show_library_manager()),
                    ),
                ],
            ];
            for (index, group) in groups.iter().enumerate() {
                if index > 0 {
                    sections.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
                }
                for button in group {
                    sections.append(button);
                }
            }
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
            player.show(player.page.get());
            player.reopen_preferences();
        }
    });
    player.window.add_action(&language_action);
}

/// Window actions and keyboard shortcuts, and saving when the window closes.
fn install_window_actions(app: &adw::Application, player: &Rc<Player>) {
    let close = gio::SimpleAction::new("close", None);
    let window = player.window.downgrade();
    close.connect_activate(move |_, _| {
        if let Some(window) = window.upgrade() {
            window.close();
        }
    });
    player.window.add_action(&close);
    app.set_accels_for_action("win.close", &["<Primary>q"]);
    player.install_shortcuts(app.upcast_ref());

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
}

fn connect_playback_controls(player: &Rc<Player>, previous: &gtk::Button, next: &gtk::Button) {
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
        state.repeat = state.repeat.next();
        drop(state);
        player.update_repeat();
        player.save();
    });
}

/// Polls playback and keeps timed lyrics in step while playing.
fn start_timers(player: &Rc<Player>) {
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
}

/// Restores the last session and shows the window.
fn start(player: &Rc<Player>, load_error: Option<std::io::Error>) {
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
