use std::{path::PathBuf, rc::Rc};

use gtk::{gio, prelude::*};

use crate::{Import, Page, Player, button, clear, table_label};

impl Player {
    pub fn library_toolbar(self: &Rc<Self>) -> gtk::Box {
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let manage = button(
            self.text("Manage library", "Gerenciar biblioteca"),
            self,
            Self::show_library_manager,
        );
        manage.add_css_class("flat");
        toolbar.append(&manage);
        toolbar
    }

    pub fn hide_library_tracks(self: &Rc<Self>, tracks: Vec<PathBuf>) {
        if !tracks.is_empty() {
            self.state.borrow_mut().hidden_library_tracks.extend(tracks);
            self.refresh_library_visibility();
        }
    }

    pub fn refresh_library_visibility(self: &Rc<Self>) {
        self.save();
        if self.page.get() == Page::Library {
            self.show(Page::Library);
        }
        if let Some(rows) = self.library_manager_rows.upgrade() {
            self.render_hidden_library(&rows);
        }
    }

    pub fn show_library_manager(self: &Rc<Self>) {
        let window = gtk::Window::builder()
            .transient_for(&self.window)
            .modal(true)
            .destroy_with_parent(true)
            .title(self.text("Manage library", "Gerenciar biblioteca"))
            .default_width(555)
            .default_height(375)
            .build();
        let layout = gtk::Box::new(gtk::Orientation::Vertical, 10);
        layout.add_css_class("content");

        let description = gtk::Label::new(Some(self.text(
            "Hidden tracks stay in saved playlists and in the queue.",
            "As faixas ocultas continuam nas listas de reprodução salvas e na fila.",
        )));
        description.set_wrap(true);
        description.set_xalign(0.0);
        layout.append(&description);
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 6);
        self.library_manager_rows.set(Some(&rows));
        let dialog = window.downgrade();
        let folder_buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let add_dialog = dialog.clone();
        folder_buttons.append(&button(
            self.text("Add folder", "Adicionar pasta"),
            self,
            move |player| {
                if let Some(window) = add_dialog.upgrade() {
                    player.choose_folder_from(Import::Library, &window);
                }
            },
        ));
        let hide = button(
            self.text("Hide a folder…", "Ocultar uma pasta…"),
            self,
            move |player| {
                let Some(parent) = dialog.upgrade() else {
                    return;
                };
                let picker = gtk::FileDialog::builder()
                    .title(player.text("Choose a folder", "Escolha uma pasta"))
                    .build();
                if let Some(folder) = player
                    .state
                    .borrow()
                    .tracks
                    .first()
                    .and_then(|path| path.parent())
                {
                    picker.set_initial_folder(Some(&gio::File::for_path(folder)));
                }
                let weak = Rc::downgrade(player);
                picker.select_folder(Some(&parent), gio::Cancellable::NONE, move |result| {
                    let (Some(player), Ok(folder)) = (weak.upgrade(), result) else {
                        return;
                    };
                    let Some(path) = folder.path().and_then(|path| path.canonicalize().ok()) else {
                        return;
                    };
                    player
                        .state
                        .borrow_mut()
                        .hidden_library_folders
                        .insert(path);
                    player.refresh_library_visibility();
                });
            },
        );
        hide.set_halign(gtk::Align::Start);
        folder_buttons.append(&hide);
        layout.append(&folder_buttons);
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&rows)
            .build();
        layout.append(&scroll);
        let close = gtk::Button::with_label(self.text("Close", "Fechar"));
        close.set_halign(gtk::Align::End);
        let dialog = window.downgrade();
        close.connect_clicked(move |_| {
            if let Some(dialog) = dialog.upgrade() {
                dialog.close();
            }
        });
        layout.append(&close);
        self.render_hidden_library(&rows);
        window.set_child(Some(&layout));
        window.present();
    }

    fn render_hidden_library(self: &Rc<Self>, rows: &gtk::Box) {
        clear(rows);
        let (roots, folders, tracks) = {
            let state = self.state.borrow();
            (
                state.library_roots.clone(),
                state
                    .hidden_library_folders
                    .difference(&state.library_roots)
                    .cloned()
                    .collect::<std::collections::BTreeSet<_>>(),
                state.hidden_library_tracks.clone(),
            )
        };
        if roots.is_empty() && folders.is_empty() && tracks.is_empty() {
            let text = self.text("No folders or hidden items", "Nenhuma pasta ou item oculto");
            let empty = gtk::Label::new(Some(text));
            empty.add_css_class("empty-state");
            rows.append(&empty);
            return;
        }
        for (title, is_folder, items) in [
            (
                self.text("Library folders", "Pastas da biblioteca"),
                true,
                roots,
            ),
            (self.text("Hidden folders", "Pastas ocultas"), true, folders),
            (self.text("Hidden tracks", "Faixas ocultas"), false, tracks),
        ] {
            if items.is_empty() {
                continue;
            }
            let heading = gtk::Label::new(Some(title));
            heading.set_xalign(0.0);
            heading.add_css_class("section-title");
            rows.append(&heading);
            for path in items {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let item = gtk::Box::new(gtk::Orientation::Vertical, 2);
                item.set_hexpand(true);
                let name = path
                    .file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy();
                item.append(&table_label(&name, 0));
                let location = table_label(&path.to_string_lossy(), 0);
                location.add_css_class("dim-label");
                item.append(&location);
                row.append(&item);
                let hidden = {
                    let state = self.state.borrow();
                    !is_folder || state.hidden_library_folders.contains(&path)
                };
                let restore = button(
                    if hidden {
                        self.text("Show in library", "Mostrar na biblioteca")
                    } else {
                        self.text("Hide from library", "Ocultar da biblioteca")
                    },
                    self,
                    move |player| {
                        {
                            let mut state = player.state.borrow_mut();
                            if is_folder && hidden {
                                state.hidden_library_folders.remove(&path);
                            } else if is_folder {
                                state.hidden_library_folders.insert(path.clone());
                            } else {
                                state.hidden_library_tracks.remove(&path);
                            }
                        }
                        player.refresh_library_visibility();
                    },
                );
                restore.set_valign(gtk::Align::Center);
                row.append(&restore);
                rows.append(&row);
            }
        }
    }
}
