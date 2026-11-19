use gtk::{gdk, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub const MINIMUM: [i32; 4] = [130, 60, 60, 82];
pub const MAXIMUM: i32 = 1600;

pub struct TrackColumns {
    groups: [gtk::SizeGroup; 3],
    headers: [gtk::Overlay; 4],
    titles: [gtk::Label; 4],
    reserved: i32,
    actions: gtk::SizeGroup,
    resize: Rc<RefCell<Option<([Option<i32>; 4], Option<f64>)>>>,
}

impl TrackColumns {
    pub fn new(reserved: i32) -> Self {
        Self {
            groups: std::array::from_fn(|_| gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal)),
            headers: std::array::from_fn(|_| gtk::Overlay::new()),
            titles: std::array::from_fn(|_| super::table_label("", 0)),
            reserved,
            actions: gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal),
            resize: Rc::new(RefCell::new(None)),
        }
    }

    pub fn header(
        &self,
        index: usize,
        title: &str,
        tooltip: &str,
        resized: impl Fn([Option<i32>; 4], i32, bool) + 'static,
        auto_fit: impl Fn() + 'static,
        moved: impl Fn(usize) -> bool + 'static,
        sort: Option<Box<dyn Fn()>>,
    ) -> gtk::Overlay {
        let header = &self.headers[index];
        let label = &self.titles[index];
        label.set_text(title);
        label.set_margin_end(8);
        header.set_size_request(-1, 22);
        place_label(header, label);
        let dragged_past_threshold = Rc::new(Cell::new(false));
        if let Some(sort) = sort {
            let sort = Rc::new(sort);
            let click_sort = sort.clone();
            let click = gtk::GestureClick::new();
            click.set_button(1);
            let origin = Rc::new(Cell::new(None));
            let pressed_at = origin.clone();
            let dragged = dragged_past_threshold.clone();
            click.connect_pressed(move |_, _, x, y| {
                dragged.set(false);
                pressed_at.set(Some((x, y)));
            });
            let dragged = dragged_past_threshold.clone();
            click.connect_released(move |_, _, x, y| {
                if let Some((start_x, start_y)) = origin.take() {
                    if !dragged.get() && (x - start_x).hypot(y - start_y) < 7.0 {
                        click_sort();
                    }
                }
            });
            label.add_controller(click);
            label.set_focusable(true);
            let keys = gtk::EventControllerKey::new();
            keys.connect_key_pressed(move |_, key, _, _| {
                if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space) {
                    sort();
                    return gtk::glib::Propagation::Stop;
                }
                gtk::glib::Propagation::Proceed
            });
            label.add_controller(keys);
        }
        if index == 0 {
            header.set_hexpand(true);
        } else {
            self.groups[index - 1].add_widget(header);
        }

        label.set_can_target(true);
        label.set_cursor_from_name(Some("grab"));
        let reorder = gtk::GestureDrag::new();
        reorder.set_button(1);
        reorder.connect_drag_update(move |_, dx, dy| {
            if dx.hypot(dy) >= 7.0 {
                dragged_past_threshold.set(true);
            }
        });
        let targets = self.headers.each_ref().map(|header| header.downgrade());
        let dragged = header.downgrade();
        let anchor = Rc::new(Cell::new(None));
        let initial = anchor.clone();
        reorder.connect_drag_begin(move |gesture, _, _| initial.set(pointer_x(gesture)));
        reorder.connect_drag_end(move |gesture, fallback, dy| {
            let dx = relative_x(gesture, anchor.get(), fallback);
            if dx.abs() < 10.0 || dx.abs() < dy.abs() {
                return;
            }
            let Some(source) = dragged.upgrade() else {
                return;
            };
            let Some(container) = source.parent() else {
                return;
            };
            let Some(bounds) = source.compute_bounds(&container) else {
                return;
            };
            let Some((start, _)) = gesture.start_point() else {
                return;
            };
            let x = f64::from(bounds.x()) + start + dx;
            let target = targets
                .iter()
                .enumerate()
                .filter_map(|(index, header)| {
                    let header = header.upgrade()?;
                    if !header.is_visible() {
                        return None;
                    }
                    let bounds = header.compute_bounds(&container)?;
                    let left = f64::from(bounds.x());
                    let right = left + f64::from(bounds.width());
                    let distance = if x < left {
                        left - x
                    } else {
                        (x - right).max(0.0)
                    };
                    Some((index, distance))
                })
                .min_by(|(_, left), (_, right)| left.total_cmp(right));
            if let Some((target, _)) = target {
                if target != index {
                    moved(target);
                }
            }
        });
        label.add_controller(reorder);

        let grip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        grip.add_css_class("column-grip");
        grip.set_halign(gtk::Align::End);
        grip.set_size_request(10, -1);
        grip.set_can_target(true);
        grip.set_cursor_from_name(Some("col-resize"));
        grip.set_tooltip_text(Some(tooltip));
        header.add_overlay(&grip);

        let click = gtk::GestureClick::new();
        click.set_button(1);
        click.connect_released(move |_, presses, _, _| {
            if presses == 2 {
                auto_fit();
            }
        });
        grip.add_controller(click);
        let gesture = gtk::GestureDrag::new();
        gesture.set_button(1);
        let headers = self.headers.each_ref().map(|header| header.downgrade());
        let start = self.resize.clone();
        gesture.connect_drag_begin(move |gesture, _, _| {
            let widths = headers.each_ref().map(|header| {
                header
                    .upgrade()
                    .filter(|header| header.is_visible())
                    .map(|header| header.width())
            });
            start.replace(Some((widths, pointer_x(gesture))));
        });
        let resized = Rc::new(resized);
        let start = self.resize.clone();
        let update = resized.clone();
        let moved = Rc::new(Cell::new(false));
        let dragging = moved.clone();
        gesture.connect_drag_update(move |gesture, dx, _| {
            if let Some((widths, origin)) = *start.borrow() {
                let delta = relative_x(gesture, origin, dx).round() as i32;
                if delta != 0 || dragging.get() {
                    dragging.set(true);
                    update(widths, delta, false);
                }
            }
        });
        let start = self.resize.clone();
        gesture.connect_drag_end(move |gesture, dx, _| {
            if let Some((widths, origin)) = start.replace(None) {
                let delta = relative_x(gesture, origin, dx).round() as i32;
                if moved.replace(false) || delta != 0 {
                    resized(widths, delta, true);
                }
            }
        });
        grip.add_controller(gesture);
        header.clone()
    }

    pub fn cell(&self, index: usize, text: &str) -> gtk::Overlay {
        let cell = gtk::Overlay::new();
        let label = super::table_label(text, 0);
        place_label(&cell, &label);
        if index == 0 {
            cell.set_hexpand(true);
        } else {
            self.groups[index - 1].add_widget(&cell);
            self.headers[index]
                .bind_property("visible", &cell, "visible")
                .sync_create()
                .build();
        }
        cell
    }

    pub fn set_title(&self, index: usize, title: &str) {
        self.titles[index].set_text(title);
    }

    pub fn align_actions(&self, widget: &impl IsA<gtk::Widget>) {
        self.actions.add_widget(widget);
    }

    pub fn column_visible(&self, index: usize) -> bool {
        self.headers[index].get_visible()
    }

    pub fn allocated_widths(&self) -> [Option<i32>; 4] {
        self.headers
            .each_ref()
            .map(|header| header.get_visible().then(|| header.width()))
    }

    pub fn title(&self, index: usize) -> gtk::glib::GString {
        self.titles[index].text()
    }

    pub fn fit(&self, available: i32, preferred: [i32; 3], enabled: [bool; 3]) {
        let (widths, visible) = fit_columns(available, self.reserved, preferred, enabled);
        for index in 0..3 {
            let header = &self.headers[index + 1];
            if header.width_request() != widths[index] {
                header.set_width_request(widths[index]);
            }
            if header.get_visible() != visible[index] {
                header.set_visible(visible[index]);
            }
        }
    }
}

fn pointer_x(gesture: &gtk::GestureDrag) -> Option<f64> {
    gesture
        .current_event()
        .and_then(|event| event.position())
        .map(|(x, _)| x)
}

fn relative_x(gesture: &gtk::GestureDrag, origin: Option<f64>, fallback: f64) -> f64 {
    pointer_x(gesture)
        .zip(origin)
        .map_or(fallback, |(current, origin)| current - origin)
}

pub fn resized_widths(
    mut widths: [Option<i32>; 4],
    order: [usize; 4],
    column: usize,
    delta: i32,
) -> [Option<i32>; 4] {
    let Some(position) = order.iter().position(|index| *index == column) else {
        return widths;
    };
    let Some(width) = widths[column] else {
        return widths;
    };
    let adjacent = order[position + 1..]
        .iter()
        .chain(order[..position].iter().rev())
        .find_map(|index| widths[*index].map(|width| (*index, width)));
    let Some((neighbor, neighbor_width)) = adjacent else {
        return widths;
    };
    let lower = (MINIMUM[column] - width)
        .max(neighbor_width - MAXIMUM)
        .min(0);
    let upper = (neighbor_width - MINIMUM[neighbor])
        .min(MAXIMUM - width)
        .max(0);
    let delta = delta.clamp(lower, upper);
    widths[column] = Some(width + delta);
    widths[neighbor] = Some(neighbor_width - delta);
    widths
}

pub fn auto_sized_widths(
    mut widths: [Option<i32>; 4],
    order: [usize; 4],
    column: usize,
    preferred: i32,
) -> [Option<i32>; 4] {
    let Some(position) = order.iter().position(|&index| index == column) else {
        return widths;
    };
    let Some(width) = widths[column] else {
        return widths;
    };
    let mut remaining = preferred.clamp(MINIMUM[column], MAXIMUM) - width;
    let neighbors = order[position + 1..]
        .iter()
        .copied()
        .chain(order[..position].iter().rev().copied());
    // The title already fills available space, so use it first when fitting a secondary column.
    let flex = (column != 0).then_some(0);
    let neighbors = flex
        .into_iter()
        .chain(neighbors.filter(|neighbor| Some(*neighbor) != flex));
    for neighbor in neighbors {
        let Some(current) = widths[neighbor] else {
            continue;
        };
        let change = remaining.clamp(
            (current - MAXIMUM).min(0),
            (current - MINIMUM[neighbor]).max(0),
        );
        widths[neighbor] = Some(current - change);
        remaining -= change;
        if remaining == 0 {
            break;
        }
    }
    widths[column] = Some(preferred.clamp(MINIMUM[column], MAXIMUM) - remaining);
    widths
}

fn place_label(cell: &gtk::Overlay, label: &gtk::Label) {
    label.set_valign(gtk::Align::Center);
    cell.add_overlay(label);
    cell.set_measure_overlay(label, false);
    cell.set_clip_overlay(label, true);
}

fn fit_columns(
    available: i32,
    reserved: i32,
    preferred: [i32; 3],
    mut enabled: [bool; 3],
) -> ([i32; 3], [bool; 3]) {
    let minimum_widths = [MINIMUM[1], MINIMUM[2], MINIMUM[3]];
    let mut widths =
        std::array::from_fn(|index| preferred[index].clamp(minimum_widths[index], MAXIMUM));
    let budget = (available - reserved - MINIMUM[0]).max(0);
    let mut minimum: i32 = (0..3)
        .filter(|index| enabled[*index])
        .map(|index| minimum_widths[index] + 6)
        .sum();
    // Drop secondary columns only when even their minimum widths no longer fit.
    for index in [1, 0, 2] {
        if enabled[index] && minimum > budget {
            enabled[index] = false;
            minimum -= minimum_widths[index] + 6;
        }
    }
    let extra = (budget - minimum).max(0);
    let requested: i32 = (0..3)
        .filter(|index| enabled[*index])
        .map(|index| widths[index] - minimum_widths[index])
        .sum();
    // Share reductions so a wide saved artist column cannot squeeze out the album.
    if requested > extra {
        for index in 0..3 {
            if enabled[index] {
                widths[index] = minimum_widths[index]
                    + (widths[index] - minimum_widths[index]) * extra / requested;
            }
        }
    }
    (widths, enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_uses_the_next_visible_column_and_keeps_total_width() {
        assert_eq!(
            resized_widths([Some(156), Some(125), None, Some(100)], [0, 1, 2, 3], 1, 40),
            [Some(156), Some(143), None, Some(82)]
        );
        assert_eq!(
            resized_widths(
                [Some(156), Some(125), None, Some(100)],
                [1, 0, 2, 3],
                0,
                -20
            ),
            [Some(136), Some(125), None, Some(120)]
        );
    }

    #[test]
    fn auto_fit_uses_the_title_first_and_respects_minimum_widths() {
        let widths = [Some(400), Some(125), Some(125), Some(82)];
        assert_eq!(
            auto_sized_widths(widths, [0, 1, 2, 3], 1, 320),
            [Some(205), Some(320), Some(125), Some(82)]
        );
        assert_eq!(
            auto_sized_widths(widths, [0, 1, 2, 3], 1, 800),
            [Some(130), Some(460), Some(60), Some(82)]
        );
        assert_eq!(
            auto_sized_widths(widths, [0, 1, 2, 3], 0, 140),
            [Some(140), Some(385), Some(125), Some(82)]
        );
    }

    #[test]
    fn large_saved_secondary_columns_share_space_in_a_narrow_window() {
        let (widths, shown) = fit_columns(540, 56, [300, 300, 68], [true; 3]);
        assert_eq!(shown, [true; 3]);
        assert_eq!(widths[0], widths[1]);
        assert!(widths[0] > 90);
        assert!(widths.iter().sum::<i32>() + 18 + 56 + MINIMUM[0] <= 540);
    }

    #[test]
    fn narrow_playlist_prioritizes_track_title_over_secondary_columns() {
        let (_, shown) = fit_columns(320, 143, [125, 125, 67], [true; 3]);
        assert_eq!(shown, [false; 3]);
        let (widths, shown) = fit_columns(760, 143, [190, 110, 70], [true, false, true]);
        assert_eq!(shown, [true, false, true]);
        assert_eq!(widths[0], 190);
    }
}
