use std::{collections::BTreeSet, path::PathBuf, rc::Rc};

use gtk::{gdk, gio, glib, prelude::*};

use crate::{Import, Page, Player, library, metadata, mini, shortcut_button, timestamp};

#[derive(Clone, Copy)]
enum TrackAction {
    Details,
    AddToQueue,
    HideLibrary,
    Remove,
    Folder,
    Refresh,
    Cut,
    Copy,
    Paste,
    PasteEnd,
    SelectAll,
}

impl TrackAction {
    fn icon(self) -> &'static str {
        match self {
            Self::Details => "dialog-information-symbolic",
            Self::AddToQueue => "list-add-symbolic",
            Self::HideLibrary => "view-conceal-symbolic",
            Self::Remove => "list-remove-symbolic",
            Self::Folder => "folder-open-symbolic",
            Self::Refresh => "view-refresh-symbolic",
            Self::Cut => "edit-cut-symbolic",
            Self::Copy => "edit-copy-symbolic",
            Self::Paste | Self::PasteEnd => "edit-paste-symbolic",
            Self::SelectAll => "edit-select-all-symbolic",
        }
    }
}

pub fn track_index(model: &gtk::MultiSelection, position: u32) -> Option<usize> {
    model
        .item(position)?
        .downcast::<gtk::StringObject>()
        .ok()?
        .string()
        .parse()
        .ok()
}

fn selected_indexes(model: &gtk::MultiSelection) -> Vec<usize> {
    let selected = model.selection();
    (0..selected.size())
        .filter_map(|i| {
            u32::try_from(i)
                .ok()
                .and_then(|i| track_index(model, selected.nth(i)))
        })
        .collect()
}

impl Player {
    pub fn close_track_menu(&self) {
        if let Some(popover) = self.track_popover.upgrade() {
            self.track_popover.set(None);
            popover.popdown();
            if popover.parent().is_some() {
                popover.unparent();
            }
        }
    }

    pub fn install_shortcuts(self: &Rc<Self>, app: &gtk::Application) {
        type Shortcut = (&'static str, &'static str, fn(&Rc<Player>));
        let shortcuts: [Shortcut; 12] = [
            ("open-files", "<Primary>o", |p| p.choose_files(Import::Play)),
            ("open-folder", "<Primary>k", |p| {
                p.choose_folder(Import::Play)
            }),
            ("add-library", "<Primary><Shift>k", |p| {
                p.choose_folder(Import::Library)
            }),
            (
                "manage-library",
                "<Primary><Shift>b",
                Player::show_library_manager,
            ),
            ("import-playlist", "<Primary><Shift>o", |p| {
                p.choose_files(Import::ImportPlaylist)
            }),
            ("search", "<Primary>f", |p| p.open_search_shortcut()),
            ("equalizer", "<Primary>e", Player::show_equalizer),
            ("mini", "<Primary>m", mini::open),
            ("play-pause", "<Primary>p", Player::toggle),
            ("previous", "<Alt>Left", |p| p.step(-1)),
            ("next", "<Alt>Right", |p| p.step(1)),
            ("toggle-lyrics", "<Primary>y", |p| {
                let enabled = p.state.borrow().view.lyrics;
                p.set_lyrics_enabled(!enabled);
            }),
        ];
        for (name, accelerator, callback) in shortcuts {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(player) = weak.upgrade() {
                    callback(&player);
                }
            });
            self.window.add_action(&action);
            app.set_accels_for_action(&format!("win.{name}"), &[accelerator]);
        }
    }

    pub fn attach_track_menu(
        self: &Rc<Self>,
        row: &gtk::Box,
        position: u32,
        model: &gtk::MultiSelection,
    ) {
        let click = gtk::GestureClick::new();
        click.set_button(3);
        let model = model.clone();
        let weak = Rc::downgrade(self);
        let anchor = row.downgrade();
        click.connect_pressed(move |gesture, _, x, y| {
            let (Some(player), Some(row)) = (weak.upgrade(), anchor.upgrade()) else {
                return;
            };
            if !model.is_selected(position) {
                model.select_item(position, true);
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
            player.show_track_menu(&row, &model, x as i32, y as i32);
        });
        row.add_controller(click);
    }

    pub fn attach_playlist_menu(self: &Rc<Self>, row: &gtk::Box, index: usize) {
        let click = gtk::GestureClick::new();
        click.set_button(3);
        let weak = Rc::downgrade(self);
        let anchor = row.downgrade();
        click.connect_pressed(move |gesture, _, x, y| {
            let (Some(player), Some(row)) = (weak.upgrade(), anchor.upgrade()) else {
                return;
            };
            gesture.set_state(gtk::EventSequenceState::Claimed);
            player.show_playlist_menu(&row, index, x as i32, y as i32);
        });
        row.add_controller(click);
    }

    fn show_playlist_menu(self: &Rc<Self>, row: &gtk::Box, index: usize, x: i32, y: i32) {
        let (popover, panel) = self.context_menu(row, x, y);
        let has_tracks = self
            .state
            .borrow()
            .playlists
            .get(index)
            .is_some_and(|list| !list.tracks.is_empty());
        type PlaylistAction = fn(&Rc<Player>, usize);
        let actions: [(&str, &str, &str, bool, PlaylistAction); 7] = [
            (
                "Open playlist",
                "Abrir lista de reprodução",
                "view-list-symbolic",
                true,
                |player, index| player.show(Page::Playlist(index)),
            ),
            (
                "Play playlist",
                "Tocar lista de reprodução",
                "media-playback-start-symbolic",
                has_tracks,
                |player, index| {
                    if let Some(tracks) = player.playlist_tracks(index) {
                        player.replace_queue_from(tracks, 0, Some(index));
                    }
                },
            ),
            (
                "Add to queue",
                "Adicionar à fila",
                "list-add-symbolic",
                has_tracks,
                |player, index| {
                    if let Some(tracks) = player.playlist_tracks(index) {
                        player.insert_tracks(Page::Queue, None, tracks);
                    }
                },
            ),
            (
                "Add files",
                "Adicionar arquivos",
                "document-open-symbolic",
                true,
                |player, index| player.choose_files(Import::Playlist(Some(index))),
            ),
            (
                "Rename",
                "Renomear",
                "document-edit-symbolic",
                true,
                |player, index| player.ask_name(Some(index), Vec::new()),
            ),
            (
                "Export playlist (M3U8)",
                "Exportar lista de reprodução (M3U8)",
                "document-save-as-symbolic",
                true,
                Player::export_playlist,
            ),
            (
                "Delete playlist",
                "Excluir lista de reprodução",
                "user-trash-symbolic",
                true,
                Player::confirm_delete_playlist,
            ),
        ];
        for (en, pt, icon, enabled, action) in actions {
            let control =
                self.context_action(&popover, self.text(en, pt), "", icon, move |player| {
                    action(player, index)
                });
            control.set_sensitive(enabled);
            panel.append(&control);
        }
        popover.popup();
    }

    fn playlist_tracks(&self, index: usize) -> Option<Vec<PathBuf>> {
        Some(self.state.borrow().playlists.get(index)?.tracks.clone())
    }

    pub fn attach_track_keys(self: &Rc<Self>, list: &gtk::ListView, model: &gtk::MultiSelection) {
        let controller = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(self);
        let model = model.clone();
        controller.connect_key_pressed(move |_, key, _, modifiers| {
            use gdk::{Key, ModifierType as Mod};
            let Some(player) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let ctrl = modifiers.contains(Mod::CONTROL_MASK);
            let shift = modifiers.contains(Mod::SHIFT_MASK);
            let alt = modifiers.contains(Mod::ALT_MASK);
            let editable = player.page.get() != Page::Library;
            let action = match (key.to_lower(), ctrl, shift, alt) {
                (Key::i, false, false, true) => TrackAction::Details,
                (Key::q, false, false, true) if player.page.get() == Page::Queue => {
                    TrackAction::Remove
                }
                (Key::q, false, false, true) => TrackAction::AddToQueue,
                (Key::F6, false, false, false) => TrackAction::Refresh,
                (Key::Delete, false, false, false) if editable => TrackAction::Remove,
                (Key::Delete, false, false, false) => TrackAction::HideLibrary,
                (Key::x, true, false, false) if editable => TrackAction::Cut,
                (Key::c, true, false, false) => TrackAction::Copy,
                (Key::v, true, false, false) => TrackAction::Paste,
                (Key::v, true, true, false) => TrackAction::PasteEnd,
                (Key::a, true, false, false) => TrackAction::SelectAll,
                _ => return glib::Propagation::Proceed,
            };
            player.run_track_action(action, &model);
            glib::Propagation::Stop
        });
        list.add_controller(controller);
    }

    fn show_track_menu(
        self: &Rc<Self>,
        row: &gtk::Box,
        model: &gtk::MultiSelection,
        x: i32,
        y: i32,
    ) {
        let (popover, panel) = self.context_menu(row, x, y);
        let page = self.page.get();
        let count = model.selection().size();
        for (action, en, pt, shortcut) in [
            (
                TrackAction::Details,
                "Track information",
                "Informações da faixa",
                "<Alt>i",
            ),
            (
                TrackAction::AddToQueue,
                "Add to queue",
                "Adicionar à fila",
                if page == Page::Queue { "" } else { "<Alt>q" },
            ),
            (
                TrackAction::HideLibrary,
                "Hide from library",
                "Ocultar da biblioteca",
                "Delete",
            ),
            (
                TrackAction::Remove,
                "Remove from queue",
                "Remover da fila",
                "Delete",
            ),
            (
                TrackAction::Folder,
                "Open containing folder",
                "Abrir pasta da faixa",
                "",
            ),
            (
                TrackAction::Refresh,
                "Refresh track information",
                "Atualizar informações",
                "F6",
            ),
            (
                TrackAction::Cut,
                "Cut tracks",
                "Recortar faixas",
                "<Primary>x",
            ),
            (
                TrackAction::Copy,
                "Copy tracks",
                "Copiar faixas",
                "<Primary>c",
            ),
            (
                TrackAction::Paste,
                "Paste tracks",
                "Colar faixas",
                "<Primary>v",
            ),
            (
                TrackAction::PasteEnd,
                "Paste at end",
                "Colar no fim",
                "<Primary><Shift>v",
            ),
            (
                TrackAction::SelectAll,
                "Select all tracks",
                "Selecionar todas as faixas",
                "<Primary>a",
            ),
        ] {
            if matches!(action, TrackAction::Details | TrackAction::Folder) && count != 1 {
                continue;
            }
            if matches!(action, TrackAction::Cut | TrackAction::PasteEnd) && page == Page::Library {
                continue;
            }
            if matches!(action, TrackAction::Remove) && page == Page::Library {
                continue;
            }
            if matches!(action, TrackAction::HideLibrary) && page != Page::Library {
                continue;
            }
            let label =
                if matches!(action, TrackAction::Remove) && matches!(page, Page::Playlist(_)) {
                    self.text("Remove from playlist", "Remover da lista de reprodução")
                } else {
                    self.text(en, pt)
                };
            let selection = model.clone();
            let control =
                self.context_action(&popover, label, shortcut, action.icon(), move |player| {
                    player.run_track_action(action, &selection);
                });
            panel.append(&control);
        }
        popover.popup();
    }

    fn context_menu(&self, row: &gtk::Box, x: i32, y: i32) -> (gtk::Popover, gtk::Box) {
        self.close_track_menu();
        let popover = gtk::Popover::new();
        popover.add_css_class("context-menu");
        popover.set_has_arrow(false);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x, y, 1, 1)));
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        popover.set_child(Some(&panel));
        popover.set_parent(row);
        self.track_popover.set(Some(&popover));
        popover.connect_closed(|popover| {
            if popover.parent().is_some() {
                popover.unparent();
            }
        });
        (popover, panel)
    }

    fn context_action(
        self: &Rc<Self>,
        popover: &gtk::Popover,
        label: &str,
        shortcut: &str,
        icon: &str,
        action: impl Fn(&Rc<Self>) + 'static,
    ) -> gtk::Button {
        let popup = popover.downgrade();
        let view = self.state.borrow().view;
        let icon = view.context_menu_icons.then_some(icon);
        let control = shortcut_button(
            label,
            shortcut,
            icon,
            view.context_menu_keycaps,
            self,
            move |player| {
                if let Some(popup) = popup.upgrade() {
                    popup.popdown();
                }
                action(player);
            },
        );
        control.add_css_class("context-action");
        control
    }

    fn run_track_action(self: &Rc<Self>, action: TrackAction, model: &gtk::MultiSelection) {
        if matches!(action, TrackAction::SelectAll) {
            model.select_all();
            return;
        }
        let page = self.page.get();
        let indexes = selected_indexes(model);
        let tracks: Vec<_> = {
            let state = self.state.borrow();
            let source = match page {
                Page::Library => &state.tracks,
                Page::Queue => &state.queue,
                Page::Playlist(index) => match state.playlists.get(index) {
                    Some(playlist) => &playlist.tracks,
                    None => return,
                },
                _ => return,
            };
            indexes
                .iter()
                .filter_map(|&index| source.get(index))
                .cloned()
                .collect()
        };
        match action {
            TrackAction::Details => {
                if let [path] = tracks.as_slice() {
                    self.show_track_details(path.clone());
                }
            }
            TrackAction::Folder => {
                if let [path] = tracks.as_slice() {
                    if let Some(folder) = path.parent() {
                        gtk::FileLauncher::new(Some(&gio::File::for_path(folder))).launch(
                            Some(&self.window),
                            gio::Cancellable::NONE,
                            |_| {},
                        );
                    }
                }
            }
            TrackAction::Refresh => {
                self.info
                    .borrow_mut()
                    .retain(|path, _| !tracks.contains(path));
                self.online_lyrics_cache
                    .borrow_mut()
                    .retain(|path, _| !tracks.contains(path));
                if self
                    .artwork_track
                    .borrow()
                    .as_ref()
                    .is_some_and(|path| tracks.contains(path))
                {
                    self.artwork_track.replace(None);
                    self.load_current_track();
                }
                self.scan_metadata(&tracks);
            }
            TrackAction::AddToQueue if !tracks.is_empty() => {
                self.insert_tracks(Page::Queue, None, tracks)
            }
            TrackAction::Remove => self.remove_tracks(page, indexes),
            TrackAction::HideLibrary if page == Page::Library => self.hide_library_tracks(tracks),
            TrackAction::Copy | TrackAction::Cut if !tracks.is_empty() => {
                let files: Vec<_> = tracks.iter().map(gio::File::for_path).collect();
                let data = gdk::FileList::from_array(&files);
                let provider = gdk::ContentProvider::for_value(&data.to_value());
                if self.window.clipboard().set_content(Some(&provider)).is_ok()
                    && matches!(action, TrackAction::Cut)
                {
                    self.remove_tracks(page, indexes);
                }
            }
            TrackAction::Paste | TrackAction::PasteEnd => {
                let after = if matches!(action, TrackAction::Paste) {
                    indexes.last().copied()
                } else {
                    None
                };
                let weak = Rc::downgrade(self);
                let active_model = model.clone();
                self.window.clipboard().read_value_async(
                    gdk::FileList::static_type(),
                    glib::Priority::DEFAULT,
                    gio::Cancellable::NONE,
                    move |result| {
                        let Some(player) = weak.upgrade() else { return };
                        if player.page.get() != page
                            || !player
                                .track_model
                                .upgrade()
                                .is_some_and(|model| model == active_model)
                        {
                            return;
                        }
                        if let Some(files) = result
                            .ok()
                            .and_then(|value| value.get::<gdk::FileList>().ok())
                        {
                            let paths = files
                                .files()
                                .iter()
                                .filter_map(gio::File::path)
                                .filter(|path| path.is_file() && library::supported(path))
                                .collect();
                            player.insert_tracks(page, after, paths);
                        }
                    },
                );
            }
            _ => {}
        }
    }

    fn show_track_details(self: &Rc<Self>, path: PathBuf) {
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let file = path.clone();
            let result = gio::spawn_blocking(move || metadata::read_track(&file)).await;
            let (Some(player), Ok(info)) = (weak.upgrade(), result) else {
                return;
            };
            let detail = format!(
                "{}: {}\n{}: {}\n{}: {}\n\n{}",
                player.text("Artist", "Artista"),
                info.artist.as_deref().unwrap_or("—"),
                player.text("Album", "Álbum"),
                info.album.as_deref().unwrap_or("—"),
                player.text("Duration", "Duração"),
                info.duration_ms.map(timestamp).as_deref().unwrap_or("—"),
                path.display(),
            );
            gtk::AlertDialog::builder()
                .message(info.title.unwrap_or_else(|| library::title(&path)))
                .detail(detail)
                .buttons([player.text("Close", "Fechar")])
                .modal(true)
                .build()
                .show(Some(&player.window));
        });
    }

    fn insert_tracks(self: &Rc<Self>, page: Page, after: Option<usize>, tracks: Vec<PathBuf>) {
        if tracks.is_empty() {
            return;
        }
        {
            let mut state = self.state.borrow_mut();
            match page {
                Page::Queue => {
                    state.insert_queue_tracks(after, tracks);
                }
                Page::Playlist(index) => {
                    let Some(playlist) = state.playlists.get_mut(index) else {
                        return;
                    };
                    let at = after.map_or(playlist.tracks.len(), |index| {
                        (index + 1).min(playlist.tracks.len())
                    });
                    playlist.tracks.splice(at..at, tracks);
                }
                Page::Library => {
                    drop(state);
                    self.import(tracks, Import::Library);
                    return;
                }
                _ => return,
            }
        }
        self.save();
        if self.page.get() == page {
            self.show(page);
        }
        self.load_current_track();
        self.update_transport();
    }

    pub fn remove_tracks(self: &Rc<Self>, page: Page, indexes: Vec<usize>) {
        if indexes.is_empty() {
            return;
        }
        let indexes: BTreeSet<_> = indexes.into_iter().collect();
        match page {
            Page::Queue => {
                let (was_current, next) = {
                    let mut state = self.state.borrow_mut();
                    let removed = state.remove_queue_tracks(&indexes);
                    (
                        removed,
                        state.queue.get(state.current).map(|_| state.current),
                    )
                };
                if was_current {
                    if let Some(next) = next.filter(|_| self.playing.get()) {
                        self.start(next, 0);
                        self.show(page);
                        return;
                    }
                    self.stop();
                }
                self.load_current_track();
                self.update_transport();
            }
            Page::Playlist(index) => {
                let mut state = self.state.borrow_mut();
                let Some(playlist) = state.playlists.get_mut(index) else {
                    return;
                };
                let mut index = 0;
                playlist.tracks.retain(|_| {
                    let keep = !indexes.contains(&index);
                    index += 1;
                    keep
                });
            }
            _ => return,
        }
        self.save();
        self.show(page);
    }
}
