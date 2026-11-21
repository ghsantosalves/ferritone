//! Sends what is playing to ListenBrainz, keeping listens while offline.

use super::*;
use listenbrainz::{Error, Kind, Listen};

/// How often listens saved while offline are retried.
const RETRY_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// What Ferritone knows about the ListenBrainz connection.
#[derive(Clone, PartialEq)]
pub enum Connection {
    Off,
    Checking,
    /// The user name and, once known, how many listens ListenBrainz has for them.
    Connected(String, Option<u64>),
    InvalidToken,
    Unreachable,
}

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
                    player
                        .listens_sent
                        .set(player.listens_sent.get() + batch.len());
                    player.save();
                    player.refresh_listenbrainz_status();
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
        self.verify_listenbrainz();
        self.flush_listens();
    }

    /// Checks the token in the background and records who it belongs to.
    pub(crate) fn verify_listenbrainz(self: &Rc<Self>) {
        let Some(token) = self.listenbrainz_token() else {
            self.listenbrainz_state.replace(Connection::Off);
            self.refresh_listenbrainz_status();
            return;
        };
        self.listenbrainz_state.replace(Connection::Checking);
        self.refresh_listenbrainz_status();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || {
                let user = listenbrainz::validate(&token)?;
                let count = listenbrainz::listen_count(&token, &user).ok();
                Ok((user, count))
            })
            .await
            .unwrap_or(Err(Error::Unavailable));
            let Some(player) = weak.upgrade() else { return };
            player.listenbrainz_state.replace(match result {
                Ok((user, count)) => Connection::Connected(user, count),
                Err(Error::Token) => Connection::InvalidToken,
                Err(Error::Unavailable) => Connection::Unreachable,
            });
            player.refresh_listenbrainz_status();
            // Home lists the ListenBrainz smart list only once the user is known.
            if player.page.get() == Page::Home {
                player.show(Page::Home);
            }
        });
    }

    /// Updates the status line in Preferences, if it is on screen.
    pub(crate) fn refresh_listenbrainz_status(&self) {
        let Some(label) = self
            .listenbrainz_status
            .borrow()
            .as_ref()
            .and_then(|label| label.upgrade())
        else {
            return;
        };
        let pending = self.state.borrow().pending_listens.len();
        let text = match &*self.listenbrainz_state.borrow() {
            Connection::Off => String::new(),
            Connection::Checking => self.text("Checking…", "Verificando…").to_owned(),
            Connection::Connected(user, count) => {
                let mut text = self
                    .text("Connected as {}.", "Conectado como {}.")
                    .replace("{}", user);
                if let Some(count) = count {
                    text.push(' ');
                    text.push_str(
                        &self
                            .text(
                                "Listens recorded on ListenBrainz: {}.",
                                "Escutas registradas no ListenBrainz: {}.",
                            )
                            .replace("{}", &count.to_string()),
                    );
                }
                text.push(' ');
                text.push_str(
                    &self
                        .text(
                            "Listens sent this session: {sent}. Waiting to send: {pending}.",
                            "Escutas enviadas nesta sessão: {sent}. Aguardando envio: {pending}.",
                        )
                        .replace("{sent}", &self.listens_sent.get().to_string())
                        .replace("{pending}", &pending.to_string()),
                );
                text
            }
            Connection::InvalidToken => self
                .text("This token is not valid.", "Este token não é válido.")
                .to_owned(),
            Connection::Unreachable => self
                .text(
                    "ListenBrainz could not be reached. Check the connection and that curl is installed.",
                    "Não foi possível falar com o ListenBrainz. Confira a conexão e se o curl está instalado.",
                )
                .to_owned(),
        };
        label.set_visible(!text.is_empty());
        if matches!(*self.listenbrainz_state.borrow(), Connection::Connected(..)) {
            label.add_css_class("success");
        } else {
            label.remove_css_class("success");
        }
        label.set_subtitle(&glib::markup_escape_text(&text));
    }

    pub(crate) fn listenbrainz_preferences(self: &Rc<Self>) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::new();
        group.set_title("ListenBrainz");
        group.set_description(Some(self.text(
            "Ferritone sends the artist, title, album, and length of each track you play past halfway. File paths stay on this computer.",
            "O Ferritone envia artista, título, álbum e duração de cada faixa que você ouve além da metade. Os caminhos dos arquivos ficam neste computador.",
        )));
        let (enabled, token) = {
            let state = self.state.borrow();
            (state.listenbrainz_enabled, state.listenbrainz_token.clone())
        };
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.add_css_class("page-actions");
        actions.set_valign(gtk::Align::Center);
        let check = button(
            self.text("Check token", "Verificar token"),
            self,
            |player| player.verify_listenbrainz(),
        );
        actions.append(&check);
        let link = button(
            self.text("Get your token", "Obter seu token"),
            self,
            |player| {
                gtk::UriLauncher::new("https://listenbrainz.org/settings/").launch(
                    Some(&player.window),
                    gio::Cancellable::NONE,
                    |_| {},
                );
            },
        );
        link.set_tooltip_text(Some("https://listenbrainz.org/settings/"));
        actions.append(&link);
        group.set_header_suffix(Some(&actions));
        let toggle = adw::SwitchRow::builder()
            .title(self.text(
                "Submit listens to ListenBrainz",
                "Enviar o que eu ouço ao ListenBrainz",
            ))
            .active(enabled)
            .build();
        group.add(&toggle);
        let entry = adw::PasswordEntryRow::builder()
            .title(self.text("User token", "Token do usuário"))
            .text(&token)
            .build();
        entry.update_property(&[gtk::accessible::Property::Label(self.text(
            "ListenBrainz user token",
            "Token de usuário do ListenBrainz",
        ))]);
        group.add(&entry);
        let status = adw::ActionRow::new();
        status.set_title(self.text("Status", "Estado"));
        status.set_subtitle_selectable(true);
        status.set_visible(false);
        group.add(&status);
        self.listenbrainz_status.replace(Some(status.downgrade()));
        self.refresh_listenbrainz_status();

        let weak = Rc::downgrade(self);
        toggle.connect_active_notify(move |toggle| {
            if let Some(player) = weak.upgrade() {
                player.state.borrow_mut().listenbrainz_enabled = toggle.is_active();
                player.save();
                player.verify_listenbrainz();
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
        group
    }
}
