//! Builds the Home, Library, Playlists, Queue, and playlist pages.

use super::*;

impl Player {
    pub(crate) fn home(self: &Rc<Self>) {
        self.heading(self.text("Home", "Início"));
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.add_css_class("page-actions");
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
        header.add_css_class("page-actions");
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

    pub(crate) fn new_playlist_drop_zone(self: &Rc<Self>) {
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

    pub(crate) fn playlists(self: &Rc<Self>) {
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

    pub(crate) fn playlist_entries(&self) -> Vec<(usize, String, usize)> {
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

    pub(crate) fn playlist_picker(
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

    pub(crate) fn playlist_item(
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

    pub(crate) fn style_playlist_indicator(
        &self,
        index: usize,
        row: &gtk::Box,
        indicator: &gtk::Image,
    ) {
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

    pub(crate) fn refresh_playing_playlist(&self) {
        for (index, row, indicator) in self.playlist_indicators.borrow().iter() {
            if let (Some(row), Some(indicator)) = (row.upgrade(), indicator.upgrade()) {
                self.style_playlist_indicator(*index, &row, &indicator);
            }
        }
    }

    pub(crate) fn delete_playlist_button(self: &Rc<Self>, index: usize) -> gtk::Button {
        icon_button(
            "user-trash-symbolic",
            self.text("Delete playlist", "Excluir lista de reprodução"),
            self,
            move |player| player.confirm_delete_playlist(index),
        )
    }

    pub(crate) fn confirm_delete_playlist(self: &Rc<Self>, index: usize) {
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

    pub(crate) fn choose_playlist(self: &Rc<Self>, tracks: Vec<PathBuf>) {
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

    pub(crate) fn library(self: &Rc<Self>) {
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

    pub(crate) fn library_row(
        self: &Rc<Self>,
        path: &PathBuf,
        position: usize,
    ) -> (gtk::Box, gtk::Box) {
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

    pub(crate) fn sort_column(self: &Rc<Self>, column: usize) {
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

    pub(crate) fn local_track_sort(&self) -> Option<(TrackSort, bool)> {
        self.local_sort
            .get()
            .filter(|(page, _, _)| *page == self.page.get())
            .map(|(_, sort, reverse)| (sort, reverse))
    }

    pub(crate) fn filtered_indexes(
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

    pub(crate) fn sort_library(&self, column: usize) {
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

    pub(crate) fn column_title(
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

    pub(crate) fn queue(self: &Rc<Self>) {
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

    pub(crate) fn playlist(self: &Rc<Self>, index: usize) {
        let (name, tracks) = {
            let state = self.state.borrow();
            let Some(playlist) = state.playlists.get(index) else {
                return;
            };
            (playlist.name.clone(), playlist.tracks.clone())
        };
        self.heading(&name);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.add_css_class("page-actions");
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
}
