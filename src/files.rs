//! Opening, importing, and exporting files and playlists.

use super::*;

impl Player {
    pub(crate) fn export_playlist(self: &Rc<Self>, index: usize) {
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

    pub(crate) fn accept_files(self: &Rc<Self>, widget: &impl IsA<gtk::Widget>, action: Import) {
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

    pub(crate) fn choose_folder(self: &Rc<Self>, action: Import) {
        self.choose_folder_from(action, &self.window);
    }

    pub(crate) fn choose_folder_from(
        self: &Rc<Self>,
        action: Import,
        parent: &impl IsA<gtk::Window>,
    ) {
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

    pub(crate) fn choose_files(self: &Rc<Self>, action: Import) {
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

    pub(crate) fn import(self: &Rc<Self>, paths: Vec<PathBuf>, action: Import) {
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

    pub(crate) fn ask_name(self: &Rc<Self>, index: Option<usize>, tracks: Vec<PathBuf>) {
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

    pub(crate) fn add_to_playlist(self: &Rc<Self>, index: usize, tracks: Vec<PathBuf>) {
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

    pub(crate) fn move_track(self: &Rc<Self>, index: usize, from: usize, to: usize) -> bool {
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
}
