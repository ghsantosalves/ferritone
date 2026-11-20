//! The phone remote: starting the server, applying its commands, and the QR code window.

use super::*;
use remote::{Command, Entry, LibraryEntry, PlaylistEntry, Position, Server};

/// Lists are copied to the phone every this many polls; playback state on every poll.
const LIST_SYNC_POLLS: u32 = 10;
/// Long queues are cut so the phone page stays responsive.
const MAX_REMOTE_QUEUE: usize = 1000;

impl Player {
    /// Starts the server for this computer only, or also for the network when `lan` is set.
    pub(crate) fn restart_server(&self, lan: bool) -> std::io::Result<()> {
        let token = {
            let mut state = self.state.borrow_mut();
            if state.remote_token.is_empty() {
                state.remote_token = remote::new_token();
            }
            state.remote_token.clone()
        };
        // Stop the old server first so the new one can take the same port.
        self.remote.replace(None);
        let server = Server::start(
            token,
            self.remote_shared.clone(),
            self.remote_sender.clone(),
            lan,
        )?;
        self.remote.replace(Some(server));
        self.remote_library_signature.replace(None);
        self.sync_remote(true);
        Ok(())
    }

    /// Opens the server to the phone. On failure it stays available on this computer.
    pub(crate) fn start_remote(&self) -> std::io::Result<()> {
        let result = self.restart_server(true);
        if result.is_err() {
            let _ = self.restart_server(false);
        }
        result
    }

    /// Closes the server to the network, keeping it for this computer's browser.
    pub(crate) fn stop_remote(&self) {
        if let Err(error) = self.restart_server(false) {
            eprintln!("Could not start the local remote page: {error}");
            self.remote.replace(None);
        }
    }

    fn remote_on_network(&self) -> bool {
        self.remote.borrow().as_ref().is_some_and(Server::lan)
    }

    /// The link for the QR code, if the phone remote is on and the computer is on a network.
    pub(crate) fn remote_url(&self) -> Option<String> {
        let server = self.remote.borrow();
        let server = server.as_ref().filter(|server| server.lan())?;
        Some(server.url(remote::local_ip()?, &self.state.borrow().remote_token))
    }

    /// The address for this computer's own browser, which needs no token.
    pub(crate) fn remote_local_url(&self) -> Option<String> {
        Some(self.remote.borrow().as_ref()?.local_url())
    }

    /// Called from `poll`: applies phone commands and refreshes what the phone shows.
    pub(crate) fn poll_remote(self: &Rc<Self>) {
        if self.remote.borrow().is_none() {
            return;
        }
        while let Ok(command) = self.remote_receiver.try_recv() {
            self.apply_remote(command);
        }
        let tick = self.remote_ticks.get().wrapping_add(1);
        self.remote_ticks.set(tick);
        let lists = tick % LIST_SYNC_POLLS == 0;
        if lists {
            // Tracks the desktop has not shown yet still need their titles for the phone.
            let queue: Vec<_> = self
                .state
                .borrow()
                .queue
                .iter()
                .take(MAX_REMOTE_QUEUE)
                .cloned()
                .collect();
            self.scan_metadata(&queue);
            self.sync_remote_library();
        }
        self.sync_remote(lists);
    }

    fn apply_remote(self: &Rc<Self>, command: Command) {
        match command {
            Command::Toggle => self.toggle(),
            Command::Next => self.step(1),
            Command::Previous => self.step(-1),
            Command::Seek(position_ms) => self.seek(position_ms as f64 / 1000.0),
            Command::Volume(volume) => self.volume.set_value(volume.clamp(0.0, 1.0) * 100.0),
            Command::Shuffle => self.shuffle.set_active(!self.shuffle.is_active()),
            Command::Repeat => self.repeat.emit_clicked(),
            Command::PlayQueue(index) => {
                if index < self.state.borrow().queue.len() {
                    self.start(index, 0);
                }
            }
            Command::Pause => {
                if self.playing.get() {
                    self.toggle();
                }
            }
            Command::PlayQueueAt(Position { index, position_ms }) => {
                if index < self.state.borrow().queue.len() {
                    self.start(index, position_ms);
                }
            }
            Command::PlayLibrary(index) => {
                if let Some(track) = self.remote_library_track(index) {
                    self.play_library_track(&track);
                }
            }
            Command::PlayLibraryAt(Position { index, position_ms }) => {
                if let Some(track) = self.remote_library_track(index) {
                    self.play_library_track(&track);
                    if position_ms > 0 {
                        let current = self.state.borrow().current;
                        self.start(current, position_ms);
                    }
                }
            }
            Command::PlayPlaylist(index) => {
                let tracks = self
                    .state
                    .borrow()
                    .playlists
                    .get(index)
                    .map(|playlist| playlist.tracks.clone());
                if let Some(tracks) = tracks {
                    self.replace_queue_from(tracks, 0, Some(index));
                }
            }
        }
    }

    fn remote_library_track(&self, index: usize) -> Option<PathBuf> {
        self.remote_shared
            .library
            .lock()
            .ok()?
            .paths
            .get(index)
            .cloned()
    }

    /// Rebuilds the library list for the phone when the library or its tags change.
    fn sync_remote_library(self: &Rc<Self>) {
        let tracks: Vec<PathBuf> = {
            let state = self.state.borrow();
            state
                .tracks
                .iter()
                .filter(|track| state.library_track_visible(track))
                .cloned()
                .collect()
        };
        // Cheap to compare every couple of seconds; changes when tracks or tags arrive.
        let signature = (
            tracks.len(),
            tracks.first().cloned(),
            self.info.borrow().len(),
        );
        if self.remote_library_signature.borrow().as_ref() == Some(&signature) {
            return;
        }
        self.scan_metadata(&tracks);
        let info = self.info.borrow();
        let albums = albums::group_albums(&tracks, &info, library_collation_key);
        let paths: Vec<PathBuf> = albums.into_iter().flat_map(|album| album.tracks).collect();
        let entries = paths
            .iter()
            .map(|path| {
                let metadata = info.get(path);
                LibraryEntry {
                    title: metadata
                        .and_then(|info| info.title.clone())
                        .unwrap_or_else(|| title(path)),
                    artist: metadata
                        .and_then(|info| info.artist.clone())
                        .unwrap_or_default(),
                    album: metadata
                        .and_then(|info| info.album.clone())
                        .unwrap_or_default(),
                }
            })
            .collect();
        drop(info);
        let version = match self.remote_shared.library.lock() {
            Ok(mut library) => {
                library.version += 1;
                library.entries = entries;
                library.paths = paths;
                library.version
            }
            Err(_) => return,
        };
        if let Ok(mut snapshot) = self.remote_shared.snapshot.lock() {
            snapshot.library_version = version;
        }
        self.remote_library_signature.replace(Some(signature));
    }

    fn sync_remote(&self, lists: bool) {
        let state = self.state.borrow();
        let info = self.info.borrow();
        let entry = |path: &PathBuf| {
            let metadata = info.get(path);
            Entry {
                title: metadata
                    .and_then(|info| info.title.clone())
                    .unwrap_or_else(|| title(path)),
                artist: metadata
                    .and_then(|info| info.artist.clone())
                    .unwrap_or_default(),
            }
        };
        let track = state.queue.get(state.current);
        let metadata = track.and_then(|path| info.get(path));
        let duration_ms = self
            .pipeline
            .query_duration::<gst::ClockTime>()
            .map(|duration| duration.mseconds())
            .filter(|duration| *duration > 0)
            .or_else(|| metadata.and_then(|info| info.duration_ms))
            .unwrap_or(0);
        if let Ok(mut current) = self.remote_shared.current_path.lock() {
            if current.as_ref() != track {
                *current = track.cloned();
            }
        }
        let palette = self.palette();
        let Ok(mut snapshot) = self.remote_shared.snapshot.lock() else {
            return;
        };
        if snapshot.palette != palette {
            snapshot.palette = palette;
        }
        snapshot.language = match state.language.resolve() {
            Language::Portuguese => "pt",
            Language::Spanish => "es",
            Language::French => "fr",
            _ => "en",
        }
        .to_owned();
        let current = track.map(entry).unwrap_or_default();
        snapshot.title = current.title;
        snapshot.artist = current.artist;
        snapshot.album = metadata
            .and_then(|info| info.album.clone())
            .unwrap_or_default();
        snapshot.playing = self.playing.get();
        snapshot.position_ms = state.position_ms;
        snapshot.duration_ms = duration_ms;
        snapshot.volume = self.volume.value() / 100.0;
        snapshot.shuffle = state.shuffle;
        snapshot.repeat = match state.repeat {
            Repeat::Off => "off",
            Repeat::Queue => "queue",
            Repeat::Track => "track",
        }
        .to_owned();
        snapshot.current = track.map(|_| state.current);
        if lists {
            if let Ok(mut paths) = self.remote_shared.queue_paths.lock() {
                *paths = state.queue.iter().take(MAX_REMOTE_QUEUE).cloned().collect();
            }
            snapshot.queue = state
                .queue
                .iter()
                .take(MAX_REMOTE_QUEUE)
                .map(entry)
                .collect();
            snapshot.playlists = state
                .playlists
                .iter()
                .map(|playlist| PlaylistEntry {
                    name: playlist.name.clone(),
                    tracks: playlist.tracks.len(),
                })
                .collect();
        }
    }

    pub(crate) fn show_remote(self: &Rc<Self>) {
        let window = gtk::Window::builder()
            .title(self.text("Phone remote", "Controle pelo celular"))
            .transient_for(&self.window)
            .modal(true)
            .resizable(false)
            .default_width(360)
            .build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 14);
        content.set_margin_top(18);
        content.set_margin_bottom(18);
        content.set_margin_start(18);
        content.set_margin_end(18);

        let toggle_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let label = gtk::Label::new(Some(self.text(
            "Allow control from a phone",
            "Permitir controle pelo celular",
        )));
        label.set_xalign(0.0);
        label.set_hexpand(true);
        let switch = gtk::Switch::new();
        switch.set_valign(gtk::Align::Center);
        switch.set_active(self.remote_on_network());
        switch.update_property(&[gtk::accessible::Property::Label(self.text(
            "Allow control from a phone",
            "Permitir controle pelo celular",
        ))]);
        toggle_row.append(&label);
        toggle_row.append(&switch);
        content.append(&toggle_row);

        let details = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.append(&details);
        window.set_child(Some(&content));

        let fill: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::default();
        let weak = Rc::downgrade(self);
        let fill_details = {
            let details = details.downgrade();
            move || {
                let (Some(player), Some(details)) = (weak.upgrade(), details.upgrade()) else {
                    return;
                };
                clear(&details);
                let hint = |text: &str| {
                    let label = gtk::Label::new(Some(text));
                    label.set_wrap(true);
                    label.set_max_width_chars(42);
                    label.set_xalign(0.0);
                    label.add_css_class("dim-label");
                    label
                };
                let open_here = button(
                    player.text("Open on this computer", "Abrir neste computador"),
                    &player,
                    |player| {
                        if let Some(url) = player.remote_local_url() {
                            gtk::UriLauncher::new(&url).launch(
                                Some(&player.window),
                                gio::Cancellable::NONE,
                                |_| {},
                            );
                        }
                    },
                );
                open_here.set_tooltip_text(Some(player.text(
                    "Open the remote page in this computer's browser",
                    "Abre a página do controle no navegador deste computador",
                )));
                // This computer's browser can always use the page while Ferritone is open.
                let local = gtk::Box::new(gtk::Orientation::Vertical, 8);
                if let Some(address) = player.remote_local_url() {
                    local.append(&hint(&player
                        .text(
                            "On this computer, open {} in any browser while Ferritone is open.",
                            "Neste computador, abra {} em qualquer navegador enquanto o Ferritone estiver aberto.",
                        )
                        .replace("{}", address.trim_end_matches('/'))));
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    row.add_css_class("page-actions");
                    row.append(&open_here);
                    local.append(&row);
                }
                if !player.remote_on_network() {
                    details.append(&hint(player.text(
                        "When this is on, a phone on the same Wi-Fi network can play, pause, skip tracks, and choose playlists from its browser. Nothing goes through the internet.",
                        "Com isto ligado, um celular na mesma rede Wi-Fi pode tocar, pausar, trocar de faixa e escolher playlists pelo navegador. Nada passa pela internet.",
                    )));
                    details.append(&local);
                    return;
                }
                let Some(url) = player.remote_url() else {
                    details.append(&hint(player.text(
                        "This computer is not connected to a network. Connect it to Wi-Fi or a cable and open this window again.",
                        "Este computador não está conectado a uma rede. Conecte-o ao Wi-Fi ou a um cabo e abra esta janela de novo.",
                    )));
                    details.append(&local);
                    return;
                };
                let code = remote::qr_area(&url, 240);
                code.set_halign(gtk::Align::Center);
                code.update_property(&[gtk::accessible::Property::Label(player.text(
                    "QR code for the phone remote",
                    "QR code do controle pelo celular",
                ))]);
                details.append(&code);
                details.append(&hint(player.text(
                    "Scan the code with your phone's camera. The phone must be on the same Wi-Fi network as this computer.",
                    "Aponte a câmera do celular para o código. O celular precisa estar na mesma rede Wi-Fi que este computador.",
                )));
                let link = gtk::Label::new(Some(&url));
                link.set_selectable(true);
                link.set_wrap(true);
                link.set_wrap_mode(gtk::pango::WrapMode::Char);
                link.set_xalign(0.0);
                link.add_css_class("caption");
                details.append(&link);
                let reset = button(player.text("New link", "Novo link"), &player, {
                    let details = details.downgrade();
                    move |player| {
                        // A new token disconnects every phone that has the old link.
                        player.state.borrow_mut().remote_token = remote::new_token();
                        player.save();
                        if let Err(error) = player.start_remote() {
                            eprintln!("Could not restart the phone remote: {error}");
                        }
                        if let Some(details) = details.upgrade() {
                            let _ = details.activate_action("remote.refresh", None);
                        }
                    }
                });
                reset.set_tooltip_text(Some(player.text(
                    "Disconnect phones that have the current link",
                    "Desconecta os celulares que têm o link atual",
                )));
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                row.add_css_class("page-actions");
                row.append(&reset);
                details.append(&row);
                details.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
                details.append(&local);
            }
        };
        fill_details();
        fill.replace(Some(Box::new(fill_details)));

        let actions = gio::SimpleActionGroup::new();
        let refresh = gio::SimpleAction::new("refresh", None);
        let refill = fill.clone();
        refresh.connect_activate(move |_, _| {
            if let Some(fill) = refill.borrow().as_ref() {
                fill();
            }
        });
        actions.add_action(&refresh);
        window.insert_action_group("remote", Some(&actions));

        let weak = Rc::downgrade(self);
        let refill = fill.clone();
        switch.connect_state_set(move |switch, enabled| {
            let Some(player) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if enabled {
                if let Err(error) = player.start_remote() {
                    eprintln!("Could not start the phone remote: {error}");
                    player.message(player.text(
                        "The phone remote could not start.",
                        "Não foi possível ligar o controle pelo celular.",
                    ));
                    switch.set_active(false);
                    return glib::Propagation::Stop;
                }
            } else {
                player.stop_remote();
            }
            player.state.borrow_mut().remote_enabled = enabled;
            player.save();
            if let Some(fill) = refill.borrow().as_ref() {
                fill();
            }
            glib::Propagation::Proceed
        });
        window.present();
    }
}
