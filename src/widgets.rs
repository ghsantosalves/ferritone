//! Small widget builders shared by the pages.

use super::*;

pub(crate) fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

pub(crate) fn button(
    label: &str,
    player: &Rc<Player>,
    callback: impl Fn(&Rc<Player>) + 'static,
) -> gtk::Button {
    let result = gtk::Button::with_label(label);
    let player = Rc::downgrade(player);
    result.connect_clicked(move |_| {
        if let Some(player) = player.upgrade() {
            callback(&player);
        }
    });
    result
}

pub(crate) fn shortcut_button(
    label: &str,
    accelerator: &str,
    icon: Option<&str>,
    keycaps: bool,
    player: &Rc<Player>,
    callback: impl Fn(&Rc<Player>) + 'static,
) -> gtk::Button {
    let control = button("", player, callback);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    if let Some(icon) = icon {
        let image = gtk::Image::from_icon_name(icon);
        image.set_pixel_size(13);
        content.append(&image);
    }
    let text = gtk::Label::new(Some(label));
    text.set_xalign(0.0);
    text.set_hexpand(true);
    content.append(&text);
    if let Some((key, modifiers)) =
        gtk::accelerator_parse(accelerator).filter(|_| !accelerator.is_empty())
    {
        let shortcut = if keycaps {
            let widget = gtk::ShortcutLabel::new(accelerator);
            widget.add_css_class("menu-shortcut-keys");
            widget.upcast::<gtk::Widget>()
        } else {
            let widget = gtk::Label::new(Some(&gtk::accelerator_get_label(key, modifiers)));
            widget.add_css_class("menu-shortcut");
            widget.upcast::<gtk::Widget>()
        };
        shortcut.set_margin_start(8);
        content.append(&shortcut);
    }
    control.set_child(Some(&content));
    control
}

pub(crate) fn icon_button(
    icon: &str,
    tooltip: &str,
    player: &Rc<Player>,
    callback: impl Fn(&Rc<Player>) + 'static,
) -> gtk::Button {
    let control = button("", player, callback);
    control.set_icon_name(icon);
    control.set_tooltip_text(Some(tooltip));
    control.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    control.add_css_class("row-action");
    control
}

pub(crate) fn table_label(text: &str, width: i32) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_width_request(width);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    if !text.is_empty() {
        label.set_has_tooltip(true);
        label.connect_query_tooltip(|label, _, _, _, tooltip| {
            if !label.layout().is_ellipsized() {
                return false;
            }
            tooltip.set_text(Some(&label.text()));
            true
        });
    }
    label
}

pub(crate) fn style_row(row: &impl IsA<gtk::Widget>, position: usize) {
    row.add_css_class("table-row");
    if position % 2 == 1 {
        row.add_css_class("alternate");
    }
}

pub(crate) fn set_visible(widget: &impl IsA<gtk::Widget>, visible: bool) {
    if widget.get_visible() != visible {
        widget.set_visible(visible);
    }
}

pub(crate) fn scale_image(
    pixbuf: &gtk::gdk_pixbuf::Pixbuf,
    side: i32,
) -> Option<gtk::gdk_pixbuf::Pixbuf> {
    let scale = f64::from(side) / f64::from(pixbuf.width().max(pixbuf.height()));
    let width = (f64::from(pixbuf.width()) * scale).round().max(1.0) as i32;
    let height = (f64::from(pixbuf.height()) * scale).round().max(1.0) as i32;
    pixbuf.scale_simple(width, height, gtk::gdk_pixbuf::InterpType::Bilinear)
}

pub(crate) fn replace_indexes(model: &gtk::StringList, indexes: impl IntoIterator<Item = usize>) {
    let indexes: Vec<_> = indexes.into_iter().map(|index| index.to_string()).collect();
    let items: Vec<_> = indexes.iter().map(String::as_str).collect();
    model.splice(0, model.n_items(), &items);
}
