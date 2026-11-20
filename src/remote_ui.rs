//! The phone remote: starting the server, applying its commands, and the QR code window.

use super::*;
use remote::{Command, Entry, PlaylistEntry, Server};

/// Lists are copied to the phone every this many polls; playback state on every poll.
const LIST_SYNC_POLLS: u32 = 10;
/// Long queues are cut so the phone page stays responsive.
const MAX_REMOTE_QUEUE: usize = 1000;

impl Player {
    pub(crate) fn start_remote(&self) -> std::io::Result<()> {
        if self.remote.borrow().is_some() {
            return Ok(());
        }
        let token = {
            let mut state = self.state.borrow_mut();
            if state.remote_token.is_empty() {
                state.remote_token = remote::new_token();
            }
            state.remote_token.clone()
        };
        let server = Server::start(
            token,
            self.remote_shared.clone(),
            self.remote_sender.clone(),
        )?;
        self.remote.replace(Some(server));
        self.remote_cover.replace(None);
        self.sync_remote(true);
        Ok(())
    }

    pub(crate) fn stop_remote(&self) {
        self.remote.replace(None);
    }

    /// The link for the QR code, if the server runs and the computer is on a network.
    pub(crate) fn remote_url(&self) -> Option<String> {
        let server = self.remote.borrow();
        Some(
            server
                .as_ref()?
                .url(remote::local_ip()?, &self.state.borrow().remote_token),
        )
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
        let cover = self.cover.borrow().clone();
        let cover_changed = *self.remote_cover.borrow() != cover;
        if cover_changed {
            let png = cover
                .as_ref()
                .map(|texture| texture.save_to_png_bytes().to_vec());
            if let Ok(mut shared) = self.remote_shared.cover.lock() {
                *shared = png;
            }
            self.remote_cover.replace(cover);
        }
        let Ok(mut snapshot) = self.remote_shared.snapshot.lock() else {
            return;
        };
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
        if cover_changed {
            snapshot.cover = snapshot.cover.wrapping_add(1);
        }
        if lists {
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
        switch.set_active(self.remote.borrow().is_some());
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
                if player.remote.borrow().is_none() {
                    details.append(&hint(player.text(
                        "When this is on, a phone on the same Wi-Fi network can play, pause, skip tracks, and choose playlists from its browser.",
                        "Com isto ligado, um celular na mesma rede Wi-Fi pode tocar, pausar, trocar de faixa e escolher playlists pelo navegador.",
                    )));
                    return;
                }
                let Some(url) = player.remote_url() else {
                    details.append(&hint(player.text(
                        "This computer is not connected to a network. Connect it to Wi-Fi or a cable and open this window again.",
                        "Este computador não está conectado a uma rede. Conecte-o ao Wi-Fi ou a um cabo e abra esta janela de novo.",
                    )));
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
                        player.stop_remote();
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
                reset.set_halign(gtk::Align::Start);
                details.append(&reset);
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
