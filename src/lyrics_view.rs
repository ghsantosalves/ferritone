use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::{gdk, prelude::*};

use crate::{Player, button, lyrics::Lyrics};

struct VerseDrag {
    track: PathBuf,
    times: Vec<u64>,
    start: usize,
    origin_y: Option<f64>,
}

impl Player {
    pub fn attach_lyric_sync(self: &Rc<Self>, anchor: &impl IsA<gtk::Widget>) {
        let anchor = anchor.as_ref();
        anchor.set_cursor_from_name(Some("grab"));
        anchor.update_property(&[gtk::accessible::Property::Description(self.text(
            "Drag the lyrics to align the current verse with the song.",
            "Arraste a letra para alinhar o verso atual à música.",
        ))]);
        let drag = gtk::GestureDrag::new();
        drag.set_button(1);
        let state: Rc<RefCell<Option<VerseDrag>>> = Rc::new(RefCell::new(None));
        let weak = Rc::downgrade(self);
        let start = state.clone();
        drag.connect_drag_begin(move |gesture, _, _| {
            let Some(player) = weak.upgrade() else { return };
            player.update_position();
            let state = player.state.borrow();
            let Some(track) = state.queue.get(state.current) else {
                return;
            };
            let Lyrics::Timed(lines) = &*player.lyrics.borrow() else {
                return;
            };
            let mut times: Vec<_> = lines
                .iter()
                .filter(|(_, text)| !text.is_empty())
                .map(|(time, _)| *time)
                .collect();
            times.dedup();
            if times.is_empty() {
                return;
            }
            let advance = state.lyric_offsets_ms.get(track).copied().unwrap_or(0);
            let position = i128::from(state.position_ms) + i128::from(advance);
            let current = times
                .partition_point(|&time| i128::from(time) <= position)
                .saturating_sub(1);
            start.replace(Some(VerseDrag {
                track: track.clone(),
                times,
                start: current,
                origin_y: pointer_y(gesture),
            }));
        });
        let weak = Rc::downgrade(self);
        let movement = state.clone();
        drag.connect_drag_update(move |gesture, _, dy| {
            let Some(player) = weak.upgrade() else { return };
            let data = movement.borrow();
            let Some(drag) = data.as_ref() else { return };
            if !player.is_current_lyric_track(&drag.track) {
                return;
            }
            // Use surface coordinates so changing the lyric height does not add to the drag delta.
            let delta = pointer_y(gesture)
                .zip(drag.origin_y)
                .map_or(dy, |(y, origin)| y - origin);
            let step = (-delta / 22.0).trunc() as isize;
            if step == 0 && player.lyric_drag_preview.get().is_none() {
                return;
            }
            let verse = drag
                .start
                .saturating_add_signed(step)
                .min(drag.times.len() - 1);
            player.lyric_drag_preview.set(Some(drag.times[verse]));
            player.update_lyrics();
        });
        let weak = Rc::downgrade(self);
        let finish = state.clone();
        drag.connect_drag_end(move |_, _, _| {
            let data = finish.take();
            let Some(player) = weak.upgrade() else { return };
            let preview = player.lyric_drag_preview.take();
            if let (Some(drag), Some(time)) = (data, preview) {
                if player.is_current_lyric_track(&drag.track) {
                    player.update_position();
                    let playback = player.state.borrow().position_ms;
                    let offset = (i128::from(time) - i128::from(playback))
                        .clamp(i128::from(i32::MIN), i128::from(i32::MAX))
                        as i32;
                    player.set_lyric_offset(&drag.track, offset);
                    return;
                }
            }
            player.update_lyrics();
        });
        let weak = Rc::downgrade(self);
        drag.connect_cancel(move |_, _| {
            state.take();
            if let Some(player) = weak.upgrade() {
                if player.lyric_drag_preview.take().is_some() {
                    player.update_lyrics();
                }
            }
        });
        anchor.add_controller(drag);

        let click = gtk::GestureClick::new();
        click.set_button(3);
        let weak = Rc::downgrade(self);
        let widget = anchor.downgrade();
        click.connect_pressed(move |gesture, _, x, y| {
            let (Some(player), Some(anchor)) = (weak.upgrade(), widget.upgrade()) else {
                return;
            };
            if matches!(*player.lyrics.borrow(), Lyrics::Timed(_)) {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                player.lyric_sync_menu(&anchor, x as i32, y as i32);
            }
        });
        anchor.add_controller(click);
    }

    fn is_current_lyric_track(&self, track: &Path) -> bool {
        let state = self.state.borrow();
        state
            .queue
            .get(state.current)
            .is_some_and(|current| current == track)
    }

    fn set_lyric_offset(&self, track: &Path, offset: i32) {
        {
            let mut state = self.state.borrow_mut();
            if offset == 0 {
                state.lyric_offsets_ms.remove(track);
            } else {
                state.lyric_offsets_ms.insert(track.to_owned(), offset);
            }
        }
        self.update_lyrics();
        self.save();
    }

    fn lyric_sync_menu(self: &Rc<Self>, anchor: &gtk::Widget, x: i32, y: i32) {
        let (track, offset) = {
            let state = self.state.borrow();
            let Some(track) = state.queue.get(state.current) else {
                return;
            };
            (
                track.clone(),
                state.lyric_offsets_ms.get(track).copied().unwrap_or(0),
            )
        };
        self.close_track_menu();
        let popover = gtk::Popover::new();
        popover.add_css_class("context-menu");
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x, y, 1, 1)));
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 5);
        let heading = gtk::Label::new(Some(self.text("Lyric timing", "Sincronia da letra")));
        heading.add_css_class("section-title");
        panel.append(&heading);
        let status = if offset == 0 {
            self.text("Original timing", "Sincronia original")
                .to_owned()
        } else {
            let direction = if offset > 0 {
                self.text("Advanced", "Adiantada")
            } else {
                self.text("Delayed", "Atrasada")
            };
            format!("{} · {} ms", direction, offset.unsigned_abs())
        };
        panel.append(&gtk::Label::new(Some(&status)));
        let popup = popover.downgrade();
        let reset = button(
            self.text("Reset timing", "Restaurar sincronia"),
            self,
            move |player| {
                if player.is_current_lyric_track(&track) {
                    player.set_lyric_offset(&track, 0);
                }
                if let Some(popover) = popup.upgrade() {
                    popover.popdown();
                }
            },
        );
        reset.set_sensitive(offset != 0);
        reset.add_css_class("context-action");
        panel.append(&reset);
        popover.set_child(Some(&panel));
        popover.set_parent(anchor);
        self.track_popover.set(Some(&popover));
        popover.connect_closed(|popover| {
            if popover.parent().is_some() {
                popover.unparent();
            }
        });
        popover.popup();
    }
}

fn pointer_y(gesture: &gtk::GestureDrag) -> Option<f64> {
    gesture
        .current_event()
        .and_then(|event| event.position())
        .map(|(_, y)| y)
}
