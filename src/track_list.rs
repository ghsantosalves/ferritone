//! The virtualized track tables, their rows, columns, and highlight.

use super::*;

impl Player {
    pub(crate) fn remember_track_row(&self, index: usize, row: &gtk::Box) {
        if self.track_highlight.get() == Some(index) {
            row.add_css_class("current");
        }
        let mut rows = self.track_rows.borrow_mut();
        rows.retain(|_, row| row.upgrade().is_some());
        rows.insert(index, row.downgrade());
    }

    pub(crate) fn refresh_track_highlight(&self) {
        let state = self.state.borrow();
        let current = (|| {
            if !self.active.get() {
                return None;
            }
            let track = state.queue.get(state.current)?;
            let playlist = match self.page.get() {
                Page::Queue => return Some(state.current),
                Page::Playlist(index) => state.playlists.get(index)?,
                _ => return None,
            };
            if playlist.tracks == state.queue {
                return Some(state.current);
            }
            playlist.tracks.iter().position(|entry| entry == track)
        })();
        self.track_highlight.set(current);
        for (&position, row) in self.track_rows.borrow().iter() {
            if let Some(row) = row.upgrade() {
                if current == Some(position) {
                    row.add_css_class("current");
                } else {
                    row.remove_css_class("current");
                }
            }
        }
    }

    pub(crate) fn virtual_tracks(
        self: &Rc<Self>,
        strings: &gtk::StringList,
        bind: impl Fn(&Rc<Self>, usize, usize) -> Option<(gtk::Box, gtk::Box)> + 'static,
        activate: impl Fn(&Rc<Self>, usize) + 'static,
    ) -> gtk::ListView {
        let factory = gtk::SignalListItemFactory::new();
        let model = gtk::MultiSelection::new(Some(strings.clone()));
        let row_model = model.clone();
        let activate = Rc::new(activate);
        let row_activate = activate.clone();
        let weak = Rc::downgrade(self);
        let unbind_player = weak.clone();
        let page = self.page.get();
        factory.connect_bind(move |_, item| {
            let (Some(player), Some(item)) = (weak.upgrade(), item.downcast_ref::<gtk::ListItem>())
            else {
                return;
            };
            if player.page.get() != page || player.columns.borrow().is_none() {
                return;
            }
            let Some(index) = item
                .item()
                .and_downcast::<gtk::StringObject>()
                .and_then(|entry| entry.string().parse::<usize>().ok())
            else {
                return;
            };
            if let Some((row, title)) = bind(&player, index, item.position() as usize) {
                player.attach_track_menu(&row, item.position(), &row_model);
                let click = gtk::GestureClick::new();
                click.set_button(1);
                let activate = row_activate.clone();
                let player = Rc::downgrade(&player);
                click.connect_released(move |gesture, count, _, _| {
                    let modifiers = gesture.current_event_state();
                    if count != 1
                        || modifiers.intersects(
                            gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
                        )
                    {
                        return;
                    }
                    let weak = player.clone();
                    let activate = activate.clone();
                    glib::idle_add_local_once(move || {
                        if let Some(player) =
                            weak.upgrade().filter(|player| player.page.get() == page)
                        {
                            activate(&player, index);
                        }
                    });
                });
                title.add_controller(click);
                item.set_child(Some(&row));
            }
        });
        factory.connect_unbind(move |_, item| {
            if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
                if let (Some(player), Some(row)) = (unbind_player.upgrade(), item.child()) {
                    if player
                        .track_popover
                        .upgrade()
                        .and_then(|popover| popover.parent())
                        .as_ref()
                        == Some(&row)
                    {
                        player.close_track_menu();
                    }
                }
                item.set_child(gtk::Widget::NONE);
            }
        });
        let list = gtk::ListView::new(Some(model.clone()), Some(factory));
        list.set_show_separators(false);
        self.track_model.set(Some(&model));
        self.attach_track_keys(&list, &model);
        let weak = Rc::downgrade(self);
        list.connect_activate(move |_, position| {
            if let (Some(player), Some(index)) =
                (weak.upgrade(), track_menu::track_index(&model, position))
            {
                activate(&player, index);
            }
        });
        list.add_css_class("library-tracks");
        list
    }

    pub(crate) fn track_viewport(
        self: &Rc<Self>,
        has_grip: bool,
        actions: usize,
    ) -> gtk::ScrolledWindow {
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        header.add_css_class("track-header");
        if has_grip {
            let grip = gtk::Label::new(None);
            grip.set_width_request(30);
            header.append(&grip);
        }
        let reserved = 22 + (actions as i32 * 34) + if has_grip { 36 } else { 0 };
        let columns = TrackColumns::new(reserved);
        let page = self.page.get();
        let state = self.state.borrow();
        let order = state.view.ordered_columns();
        let marker = if page == Page::Library {
            Some((state.sort, state.descending))
        } else {
            self.local_track_sort()
        };
        drop(state);
        for index in order {
            let resize = Rc::downgrade(self);
            let fit = Rc::downgrade(self);
            let reorder = Rc::downgrade(self);
            let label = match marker {
                Some((sort, descending)) => self.column_title(index, true, sort, descending),
                None => self.column_title(index, false, TrackSort::Title, false),
            };
            let weak = Rc::downgrade(self);
            let sort = Some(Box::new(move || {
                if let Some(player) = weak.upgrade() {
                    player.sort_column(index);
                }
            }) as Box<dyn Fn()>);
            header.append(&columns.header(
                index,
                &label,
                self.text(
                    "Drag to resize or double-click to fit",
                    "Arraste para redimensionar ou dê um duplo clique para ajustar ao conteúdo",
                ),
                move |widths, delta, finished| {
                    if let Some(player) = resize.upgrade() {
                        player.resize_column(index, widths, delta, finished);
                    }
                },
                move || {
                    if let Some(player) = fit.upgrade() {
                        player.auto_fit_column(index);
                    }
                },
                move |target| {
                    let Some(player) = reorder.upgrade() else {
                        return false;
                    };
                    if !player.state.borrow_mut().view.move_column(index, target) {
                        return false;
                    }
                    player.save();
                    let weak = Rc::downgrade(&player);
                    glib::idle_add_local_once(move || {
                        if let Some(player) = weak.upgrade() {
                            player.show(player.page.get());
                        }
                    });
                    true
                },
                sort,
            ));
        }
        if actions > 0 {
            let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            columns.align_actions(&spacer);
            header.append(&spacer);
        }
        self.columns.replace(Some(columns));
        self.fit_columns();
        let table = gtk::Box::new(gtk::Orientation::Vertical, 0);
        table.append(&header);
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .overlay_scrolling(true)
            .build();
        table.append(&scroll);
        self.content.append(&table);
        scroll
    }

    pub(crate) fn track_row(
        self: &Rc<Self>,
        path: &std::path::Path,
        position: usize,
    ) -> (gtk::Box, gtk::Box) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        style_row(&row, position);
        let metadata = self.info.borrow();
        let info = metadata.get(path);
        let track = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        track.add_css_class("track-title");
        track.set_hexpand(true);
        track.set_cursor_from_name(Some("pointer"));
        let cells = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let columns = self.columns.borrow();
        let columns = columns
            .as_ref()
            .expect("track headers are created before track rows");
        for index in self.state.borrow().view.ordered_columns() {
            cells.append(&columns.cell(index, &track_cell_text(path, info, index)));
        }
        cells.set_hexpand(true);
        track.append(&cells);
        row.append(&track);
        (row, track)
    }

    pub(crate) fn resize_column(
        &self,
        index: usize,
        starting_widths: [Option<i32>; 4],
        delta: i32,
        finished: bool,
    ) {
        let order = self.state.borrow().view.ordered_columns();
        let widths = table::resized_widths(starting_widths, order, index, delta);
        self.apply_column_widths(widths, finished);
    }

    pub(crate) fn auto_fit_column(&self, column: usize) {
        let widths = {
            let columns = self.columns.borrow();
            let (Some(columns), Some(model)) = (columns.as_ref(), self.track_model.upgrade())
            else {
                return;
            };
            let state = self.state.borrow();
            let tracks = match self.page.get() {
                Page::Library => &state.tracks,
                Page::Queue => &state.queue,
                Page::Playlist(index) => match state.playlists.get(index) {
                    Some(playlist) => &playlist.tracks,
                    None => return,
                },
                _ => return,
            };
            let metadata = self.info.borrow();
            let layout = self
                .content
                .create_pango_layout(Some(&columns.title(column)));
            let mut widest = layout.pixel_size().0 + 16;
            for position in 0..model.n_items() {
                if let Some(path) =
                    track_menu::track_index(&model, position).and_then(|index| tracks.get(index))
                {
                    layout.set_text(&track_cell_text(path, metadata.get(path), column));
                    widest = widest.max(layout.pixel_size().0 + 16);
                    if widest >= table::MAXIMUM {
                        break;
                    }
                }
            }
            table::auto_sized_widths(
                columns.allocated_widths(),
                state.view.ordered_columns(),
                column,
                widest,
            )
        };
        self.apply_column_widths(widths, true);
    }

    pub(crate) fn apply_column_widths(&self, widths: [Option<i32>; 4], finished: bool) {
        {
            let mut state = self.state.borrow_mut();
            let view = &mut state.view;
            for (saved, width) in view
                .column_widths
                .iter_mut()
                .zip(widths.into_iter().skip(1))
            {
                if let Some(width) = width {
                    *saved = width;
                }
            }
        }
        self.fit_columns();
        if finished {
            self.save();
        }
    }

    pub(crate) fn track_actions(&self, row: &gtk::Box) -> gtk::Box {
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        self.columns
            .borrow()
            .as_ref()
            .expect("track headers exist")
            .align_actions(&actions);
        row.append(&actions);
        actions
    }

    pub(crate) fn reorder_controls(
        self: &Rc<Self>,
        row: &gtk::Box,
        actions: &gtk::Box,
        position: usize,
        count: usize,
        reorder: impl Fn(&Rc<Self>, usize, usize) -> bool + 'static,
    ) {
        let reorder = Rc::new(reorder);
        let drag = gtk::DragSource::builder()
            .actions(gdk::DragAction::MOVE)
            .build();
        let dragged_position = gtk::StringObject::new(&position.to_string());
        drag.set_content(Some(&gdk::ContentProvider::for_value(
            &dragged_position.to_value(),
        )));
        let handle = gtk::Label::new(Some("⠿"));
        handle.set_width_request(30);
        handle.set_can_target(true);
        handle.set_cursor_from_name(Some("grab"));
        handle.set_tooltip_text(Some(self.text("Drag to reorder", "Arraste para reordenar")));
        handle.add_controller(drag);
        row.prepend(&handle);

        let target = gtk::DropTarget::new(gtk::StringObject::static_type(), gdk::DragAction::MOVE);
        let weak = Rc::downgrade(self);
        let move_by_drag = reorder.clone();
        target.connect_drop(move |_, value, _, _| {
            let (Some(player), Ok(from)) = (weak.upgrade(), value.get::<gtk::StringObject>())
            else {
                return false;
            };
            from.string()
                .parse::<usize>()
                .is_ok_and(|from| move_by_drag(&player, from, position))
        });
        row.add_controller(target);
        let move_up = reorder.clone();
        let up = icon_button(
            "go-up-symbolic",
            self.text("Move up", "Subir"),
            self,
            move |player| {
                move_up(player, position, position.saturating_sub(1));
            },
        );
        up.set_sensitive(position > 0);
        actions.append(&up);
        let down = icon_button(
            "go-down-symbolic",
            self.text("Move down", "Descer"),
            self,
            move |player| {
                reorder(player, position, position + 1);
            },
        );
        down.set_sensitive(position + 1 < count);
        actions.append(&down);
    }

    pub(crate) fn fit_columns(&self) {
        let active = self.columns.borrow();
        let Some(columns) = active.as_ref() else {
            return;
        };
        let view = self.state.borrow().view;
        let width = self.content.width();
        // Before the first allocation, use a compact layout so the table does not set a wide window minimum.
        columns.fit(
            if width > 0 { width } else { 400 },
            view.column_widths,
            view.columns,
        );
    }
}
