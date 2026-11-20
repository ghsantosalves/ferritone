//! Search bars on the track and playlist pages.

use super::*;

impl Player {
    pub(crate) fn search_bar(&self, entry: &gtk::SearchEntry) -> SearchBar {
        SearchBar::new(
            entry,
            self.text("Previous result", "Resultado anterior"),
            self.text("Next result", "Próximo resultado"),
        )
    }

    pub(crate) fn mount_track_search(&self, entry: &gtk::SearchEntry) -> SearchBar {
        let bar = self.search_bar(entry);
        self.content.append(&bar.widget);
        self.track_search_field.replace(Some(bar.clone()));
        bar
    }

    pub(crate) fn timeline_fits(&self) -> bool {
        const MIN_TIMELINE_WIDTH: i32 = 100;
        let window_width = self.window.width();
        if matches!(window_width, 1..=760) {
            return false;
        }

        let width = self.transport.width();
        if width == 0 {
            return true;
        }

        let mut needed = MIN_TIMELINE_WIDTH;
        let mut child = self.transport.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if widget.get_visible() && widget != self.progress {
                let width = widget.measure(gtk::Orientation::Horizontal, -1).1;
                needed += width + self.transport.spacing();
            }
        }
        width >= needed
    }

    pub(crate) fn toggle_track_search(&self) {
        let Some(bar) = self.track_search_field.borrow().clone() else {
            return;
        };
        if bar.widget.get_visible() {
            self.collapse_search(&bar);
        } else {
            self.track_search_open.set(SearchVisibility::Expanded);
            self.fit_search();
            bar.entry.grab_focus();
        }
    }

    pub(crate) fn open_search_shortcut(&self) {
        let search = self.track_search_field.borrow().clone();
        if let Some(bar) = search {
            self.track_search_open.set(SearchVisibility::Shortcut);
            self.fit_search();
            bar.entry.grab_focus();
        }
    }

    pub(crate) fn collapse_search(&self, bar: &SearchBar) {
        self.track_search_open.set(SearchVisibility::Collapsed);
        bar.entry.set_text("");
        self.fit_search();
    }

    pub(crate) fn install_search_dismissal(self: &Rc<Self>) {
        let events = gtk::EventControllerLegacy::new();
        events.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        events.connect_event(move |_, event| {
            if event.event_type() != gdk::EventType::ButtonRelease {
                return glib::Propagation::Proceed;
            }
            let Some(player) = weak
                .upgrade()
                .filter(|p| p.track_search_open.get() == SearchVisibility::Shortcut)
            else {
                return glib::Propagation::Proceed;
            };
            let Some(bar) = player.track_search_field.borrow().clone() else {
                return glib::Propagation::Proceed;
            };
            let Some((x, y)) = event.position() else {
                return glib::Propagation::Proceed;
            };
            if player
                .window
                .pick(x, y, gtk::PickFlags::DEFAULT)
                .is_some_and(|picked| {
                    picked == bar.widget
                        || picked.is_ancestor(&bar.widget)
                        || picked == player.compact_search
                        || picked.is_ancestor(&player.compact_search)
                })
            {
                return glib::Propagation::Proceed;
            }
            let weak = weak.clone();
            let entry = bar.entry.downgrade();
            glib::idle_add_local_once(move || {
                let (Some(player), Some(entry)) = (weak.upgrade(), entry.upgrade()) else {
                    return;
                };
                let current = player.track_search_field.borrow().clone();
                if let Some(bar) = current.filter(|bar| bar.entry == entry) {
                    if player.track_search_open.get() == SearchVisibility::Shortcut {
                        player.collapse_search(&bar);
                    }
                }
            });
            glib::Propagation::Proceed
        });
        self.window.add_controller(events);
    }

    pub(crate) fn fit_search(&self) {
        let view = self.state.borrow().view;
        let search = self.track_search_field.borrow();
        let mut show_button = false;
        if let Some(bar) = search.as_ref() {
            let visible = match self.track_search_open.get() {
                SearchVisibility::Automatic => !view.search_button || !bar.entry.text().is_empty(),
                SearchVisibility::Expanded | SearchVisibility::Shortcut => true,
                SearchVisibility::Collapsed => false,
            };
            set_visible(&bar.widget, visible);
            show_button = view.search_button || !visible;
        }
        set_visible(&self.compact_search, show_button);
        set_visible(&self.progress, view.seek_bar && self.timeline_fits());
    }
}
