use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use glib::Unichar;
use gtk::{glib, prelude::*};

pub fn search_key(text: &str) -> String {
    glib::normalize(glib::casefold(text.trim()), glib::NormalizeMode::Default)
        .chars()
        .filter(|character| !character.is_mark())
        .collect()
}

#[derive(Clone, Copy, Default, PartialEq)]
pub enum SearchVisibility {
    #[default]
    Automatic,
    Expanded,
    Collapsed,
    Shortcut,
}

#[derive(Clone)]
pub struct SearchBar {
    pub widget: gtk::Box,
    pub entry: gtk::SearchEntry,
    pub results: Rc<SearchResults>,
}

type Navigate = Box<dyn Fn(Option<u32>, bool)>;

pub struct SearchResults {
    current: Cell<u32>,
    count: Cell<u32>,
    controls: glib::WeakRef<gtk::Box>,
    previous: glib::WeakRef<gtk::Button>,
    next: glib::WeakRef<gtk::Button>,
    counter: glib::WeakRef<gtk::Label>,
    navigate: RefCell<Option<Navigate>>,
}

impl SearchBar {
    pub fn new(entry: &gtk::SearchEntry, previous_text: &str, next_text: &str) -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        widget.add_css_class("search-bar");
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        controls.set_visible(false);
        let counter = gtk::Label::new(None);
        counter.add_css_class("search-counter");
        let previous = result_button("go-up-symbolic", previous_text);
        let next = result_button("go-down-symbolic", next_text);
        controls.append(&counter);
        controls.append(&previous);
        controls.append(&next);
        entry.set_hexpand(true);
        widget.append(entry);
        widget.append(&controls);

        let results = Rc::new(SearchResults {
            current: Cell::new(0),
            count: Cell::new(0),
            controls: controls.downgrade(),
            previous: previous.downgrade(),
            next: next.downgrade(),
            counter: counter.downgrade(),
            navigate: RefCell::new(None),
        });
        for (button, forward) in [(previous, false), (next, true)] {
            let results = results.clone();
            button.connect_clicked(move |_| results.step(forward));
        }
        Self {
            widget,
            entry: entry.clone(),
            results,
        }
    }
}

impl SearchResults {
    pub fn track_list(&self, list: &gtk::ListView) {
        let list = list.downgrade();
        self.navigate
            .replace(Some(Box::new(move |position, scroll| {
                let Some(list) = list.upgrade() else { return };
                if let (Some(model), Some(position)) = (list.model(), position) {
                    model.select_item(position, true);
                    if scroll {
                        let _ = gtk::prelude::WidgetExt::activate_action(
                            &list,
                            "list.scroll-to-item",
                            Some(&position.to_variant()),
                        );
                    } else if let Some(adjustment) = list.vadjustment() {
                        adjustment.set_value(adjustment.lower());
                    }
                }
            })));
    }

    pub fn playlist_rows(&self, rows: &gtk::Box, scroll: &gtk::ScrolledWindow) {
        let rows = rows.downgrade();
        let scroll = scroll.downgrade();
        self.navigate
            .replace(Some(Box::new(move |position, center| {
                let (Some(rows), Some(scroll)) = (rows.upgrade(), scroll.upgrade()) else {
                    return;
                };
                let adjustment = scroll.vadjustment();
                if !center {
                    adjustment.set_value(adjustment.lower());
                }
                let mut row = rows.first_child();
                let mut index = 0;
                while let Some(widget) = row {
                    row = widget.next_sibling();
                    if Some(index) == position {
                        widget.add_css_class("search-result");
                        if center {
                            if let Some(bounds) = widget.compute_bounds(&rows) {
                                let center = f64::from(bounds.y() + bounds.height() / 2.0);
                                adjustment.set_value(center - adjustment.page_size() / 2.0);
                            }
                        }
                    } else {
                        widget.remove_css_class("search-result");
                    }
                    index += 1;
                }
            })));
    }

    pub fn update(&self, text: &str, count: u32) {
        let active = !text.trim().is_empty();
        self.count.set(if active { count } else { 0 });
        self.current.set(0);
        if let Some(controls) = self.controls.upgrade() {
            controls.set_visible(active);
        }
        for button in [self.previous.upgrade(), self.next.upgrade()]
            .into_iter()
            .flatten()
        {
            button.set_sensitive(active && count > 1);
        }
        self.show_current(false);
    }

    fn step(&self, forward: bool) {
        let count = self.count.get();
        if count <= 1 {
            return;
        }
        let current = self.current.get();
        let next = if forward {
            (current + 1) % count
        } else {
            current.checked_sub(1).unwrap_or(count - 1)
        };
        self.current.set(next);
        self.show_current(true);
    }

    fn show_current(&self, scroll: bool) {
        let count = self.count.get();
        let position = (count > 0).then(|| self.current.get());
        if let Some(counter) = self.counter.upgrade() {
            counter.set_text(&format!(
                "{} / {count}",
                position.map_or(0, |index| index + 1)
            ));
        }
        if let Some(navigate) = self.navigate.borrow().as_ref() {
            navigate(position, scroll);
        }
    }
}

fn result_button(icon: &str, text: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.add_css_class("flat");
    button.set_focus_on_click(false);
    button.set_tooltip_text(Some(text));
    button.update_property(&[gtk::accessible::Property::Label(text)]);
    button
}

#[cfg(test)]
mod tests {
    use super::search_key;

    #[test]
    fn search_ignores_accents_and_case_without_losing_word_boundaries() {
        assert_eq!(search_key("  CHICO CÉSAR  "), search_key("chico cesar"));
        assert_eq!(search_key("Canção d’Água"), "cancao d’agua");
        assert_eq!(search_key("Ã"), "a");
    }
}
