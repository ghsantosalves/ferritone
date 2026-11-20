//! Sends what is playing to ListenBrainz, keeping listens while offline.

use super::*;
use listenbrainz::{Error, Kind, Listen};

/// How often listens saved while offline are retried.
const RETRY_INTERVAL: Duration = Duration::from_secs(10 * 60);

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

impl Player {
    fn listenbrainz_token(&self) -> Option<String> {
        let state = self.state.borrow();
        (state.listenbrainz_enabled && !state.listenbrainz_token.trim().is_empty())
            .then(|| state.listenbrainz_token.trim().to_owned())
    }

    /// The current track as a listen, if its tags name an artist and a title.
    fn current_listen(&self, listened_at: u64) -> Option<Listen> {
        let state = self.state.borrow();
        let track = state.queue.get(state.current)?;
        let info = self.info.borrow();
        let metadata = info.get(track)?;
        Some(Listen {
            listened_at,
            artist: metadata.artist.clone()?,
            title: metadata.title.clone()?,
            album: metadata.album.clone(),
            duration_ms: metadata.duration_ms,
        })
    }

    /// Called whenever a track starts playing.
    pub(crate) fn track_started(&self) {
        self.track_started_at.set(now_seconds());
        let Some(token) = self.listenbrainz_token() else {
            return;
        };
        // Tags may still be loading; "playing now" is a courtesy, so skip it then.
        if let Some(listen) = self.current_listen(0) {
            gio::spawn_blocking(move || {
                let _ = listenbrainz::submit(&token, Kind::PlayingNow, &[listen]);
            });
        }
    }

    /// Called once a play counts; queues the listen and tries to send it.
    pub(crate) fn scrobble_current(self: &Rc<Self>) {
        if self.listenbrainz_token().is_none() {
            return;
        }
        let Some(listen) = self.current_listen(self.track_started_at.get()) else {
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            state.pending_listens.push(listen);
            let excess = state
                .pending_listens
                .len()
                .saturating_sub(listenbrainz::MAX_PENDING);
            state.pending_listens.drain(..excess);
        }
        self.save();
        self.flush_listens();
    }

    /// Sends listens saved while offline, a batch at a time.
    pub(crate) fn flush_listens(self: &Rc<Self>) {
        let Some(token) = self.listenbrainz_token() else {
            return;
        };
        let batch: Vec<Listen> = self
            .state
            .borrow()
            .pending_listens
            .iter()
            .take(listenbrainz::BATCH)
            .cloned()
            .collect();
        if batch.is_empty() || self.listens_sending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let sent = batch.clone();
            let result = gio::spawn_blocking(move || {
                let kind = if sent.len() == 1 {
                    Kind::Single
                } else {
                    Kind::Import
                };
                listenbrainz::submit(&token, kind, &sent)
            })
            .await
            .unwrap_or(Err(Error::Unavailable));
            let Some(player) = weak.upgrade() else { return };
            player.listens_sending.set(false);
            match result {
                Ok(()) => {
                    player
                        .state
                        .borrow_mut()
                        .pending_listens
                        .retain(|listen| !batch.contains(listen));
                    player.save();
                    player.flush_listens();
                }
                Err(Error::Token) => player.message(player.text(
                    "ListenBrainz did not accept the user token. Check it in Preferences.",
                    "O ListenBrainz não aceitou o token. Confira-o nas Preferências.",
                )),
                Err(Error::Unavailable) => {}
            }
        });
    }

    pub(crate) fn start_listen_retries(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(RETRY_INTERVAL, move || {
            let Some(player) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            player.flush_listens();
            glib::ControlFlow::Continue
        });
        self.flush_listens();
    }

    pub(crate) fn listenbrainz_preferences(self: &Rc<Self>, form: &gtk::Box) {
        let title = gtk::Label::new(Some("ListenBrainz"));
        title.add_css_class("section-title");
        title.set_xalign(0.0);
        form.append(&title);
        let (enabled, token) = {
            let state = self.state.borrow();
            (state.listenbrainz_enabled, state.listenbrainz_token.clone())
        };
        let toggle = gtk::CheckButton::with_label(self.text(
            "Submit listens to ListenBrainz",
            "Enviar o que eu ouço ao ListenBrainz",
        ));
        toggle.set_active(enabled);
        form.append(&toggle);
        let entry = gtk::PasswordEntry::builder()
            .show_peek_icon(true)
            .placeholder_text(self.text("User token", "Token do usuário"))
            .text(&token)
            .build();
        entry.add_css_class("preferences-control");
        entry.update_property(&[gtk::accessible::Property::Label(self.text(
            "ListenBrainz user token",
            "Token de usuário do ListenBrainz",
        ))]);
        form.append(&entry);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let status = gtk::Label::new(None);
        status.set_xalign(0.0);
        status.set_wrap(true);
        status.add_css_class("dim-label");
        let check = {
            let entry = entry.downgrade();
            let status = status.downgrade();
            button(
                self.text("Check token", "Verificar token"),
                self,
                move |player| {
                    let (Some(entry), Some(status)) = (entry.upgrade(), status.upgrade()) else {
                        return;
                    };
                    let token = entry.text().trim().to_owned();
                    if token.is_empty() {
                        return;
                    }
                    status.set_text(player.text("Checking…", "Verificando…"));
                    let weak = Rc::downgrade(player);
                    let status = status.downgrade();
                    glib::spawn_future_local(async move {
                        let result = gio::spawn_blocking(move || listenbrainz::validate(&token))
                            .await
                            .unwrap_or(Err(Error::Unavailable));
                        let (Some(player), Some(status)) = (weak.upgrade(), status.upgrade())
                        else {
                            return;
                        };
                        status.set_text(&match result {
                            Ok(user) => player
                                .text("Connected as {}.", "Conectado como {}.")
                                .replace("{}", &user),
                            Err(Error::Token) => player
                                .text("This token is not valid.", "Este token não é válido.")
                                .to_owned(),
                            Err(Error::Unavailable) => player
                                .text(
                                    "ListenBrainz could not be reached. Check the connection and that curl is installed.",
                                    "Não foi possível falar com o ListenBrainz. Confira a conexão e se o curl está instalado.",
                                )
                                .to_owned(),
                        });
                    });
                },
            )
        };
        actions.append(&check);
        let link = gtk::LinkButton::with_label(
            "https://listenbrainz.org/settings/",
            self.text("Get your token", "Obter seu token"),
        );
        actions.append(&link);
        form.append(&actions);
        form.append(&status);
        let description = gtk::Label::new(Some(self.text(
            "Ferritone sends the artist, title, album, and length of each track you play past halfway. File paths stay on this computer.",
            "O Ferritone envia artista, título, álbum e duração de cada faixa que você ouve além da metade. Os caminhos dos arquivos ficam neste computador.",
        )));
        description.set_xalign(0.0);
        description.set_wrap(true);
        form.append(&description);

        let weak = Rc::downgrade(self);
        toggle.connect_toggled(move |toggle| {
            if let Some(player) = weak.upgrade() {
                player.state.borrow_mut().listenbrainz_enabled = toggle.is_active();
                player.save();
                player.flush_listens();
            }
        });
        let weak = Rc::downgrade(self);
        entry.connect_changed(move |entry| {
            if let Some(player) = weak.upgrade() {
                player.state.borrow_mut().listenbrainz_token = entry.text().trim().to_owned();
                player.save();
            }
        });
    }
}
