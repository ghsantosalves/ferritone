//! Play counts and the automatic lists on the Home page.

use super::*;

/// How many tracks an automatic list holds.
const SMART_LIST_LENGTH: usize = 100;

#[derive(Clone, Copy)]
enum SmartList {
    RecentlyAdded,
    RecentlyPlayed,
    MostPlayed,
}

impl Player {
    /// Called from `poll`: counts the current track once it has played long enough.
    pub(crate) fn count_play(self: &Rc<Self>) {
        if self.play_counted.get() || !self.playing.get() {
            return;
        }
        let duration_ms = self
            .pipeline
            .query_duration::<gst::ClockTime>()
            .map_or(0, |duration| duration.mseconds());
        let (track, position_ms) = {
            let state = self.state.borrow();
            (state.queue.get(state.current).cloned(), state.position_ms)
        };
        let Some(track) = track else { return };
        if !playback::counts_as_play(position_ms, duration_ms) {
            return;
        }
        self.play_counted.set(true);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        self.state.borrow_mut().record_play(&track, now);
        self.save();
        self.scrobble_current();
    }

    pub(crate) fn smart_lists(self: &Rc<Self>) {
        let (has_library, has_history) = {
            let state = self.state.borrow();
            (!state.tracks.is_empty(), !state.last_played.is_empty())
        };
        if !has_library && !has_history {
            return;
        }
        let label = gtk::Label::new(Some(self.text("Smart lists", "Listas automáticas")));
        label.set_xalign(0.0);
        label.add_css_class("section-title");
        label.set_margin_top(4);
        self.content.append(&label);
        let mut lists = Vec::new();
        if has_library {
            lists.push((
                SmartList::RecentlyAdded,
                self.text("Recently added", "Adicionadas recentemente"),
                "document-new-symbolic",
            ));
        }
        if has_history {
            lists.push((
                SmartList::RecentlyPlayed,
                self.text("Recently played", "Tocadas recentemente"),
                "document-open-recent-symbolic",
            ));
            lists.push((
                SmartList::MostPlayed,
                self.text("Most played", "Mais tocadas"),
                "starred-symbolic",
            ));
        }
        for (position, (list, name, icon)) in lists.into_iter().enumerate() {
            let item = button("", self, move |player| player.play_smart_list(list));
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            line.append(&gtk::Image::from_icon_name(icon));
            let name_label = table_label(name, 0);
            name_label.set_hexpand(true);
            line.append(&name_label);
            line.append(&gtk::Image::from_icon_name("media-playback-start-symbolic"));
            item.set_child(Some(&line));
            item.set_hexpand(true);
            item.add_css_class("track-title");
            item.set_tooltip_text(Some(self.text("Play this list", "Tocar esta lista")));
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            style_row(&row, position);
            row.append(&item);
            self.content.append(&row);
        }
    }

    fn play_smart_list(self: &Rc<Self>, list: SmartList) {
        let tracks = {
            let state = self.state.borrow();
            match list {
                SmartList::RecentlyPlayed => state.recently_played(SMART_LIST_LENGTH),
                SmartList::MostPlayed => state.most_played(SMART_LIST_LENGTH),
                SmartList::RecentlyAdded => {
                    let visible: Vec<_> = state
                        .tracks
                        .iter()
                        .filter(|track| state.library_track_visible(track))
                        .cloned()
                        .collect();
                    drop(state);
                    library::recently_added(&visible, SMART_LIST_LENGTH)
                }
            }
        };
        // Files that were moved or deleted since they were played are left out.
        let tracks: Vec<_> = tracks.into_iter().filter(|track| track.exists()).collect();
        if tracks.is_empty() {
            self.message(self.text(
                "There are no tracks in this list yet.",
                "Ainda não há faixas nesta lista.",
            ));
            return;
        }
        self.replace_queue(tracks, 0);
        self.show(Page::Queue);
    }
}
