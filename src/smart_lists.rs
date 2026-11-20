//! Play counts and the automatic lists on the Home page.

use super::*;

/// How many tracks an automatic list holds.
const SMART_LIST_LENGTH: usize = 100;

#[derive(Clone, Copy)]
enum SmartList {
    RecentlyAdded,
    RecentlyPlayed,
    MostPlayed,
    /// The user's most played tracks according to ListenBrainz, found in the library.
    ListenBrainz,
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
        let listenbrainz = matches!(
            *self.listenbrainz_state.borrow(),
            scrobble::Connection::Connected(..)
        );
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
        if listenbrainz && has_library {
            lists.push((
                SmartList::ListenBrainz,
                self.text(
                    "Most played on ListenBrainz",
                    "Mais ouvidas no ListenBrainz",
                ),
                "network-transmit-receive-symbolic",
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
        if matches!(list, SmartList::ListenBrainz) {
            return self.play_listenbrainz_favorites();
        }
        let tracks = {
            let state = self.state.borrow();
            match list {
                SmartList::RecentlyPlayed => state.recently_played(SMART_LIST_LENGTH),
                SmartList::MostPlayed => state.most_played(SMART_LIST_LENGTH),
                SmartList::ListenBrainz => unreachable!("handled above"),
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

impl Player {
    /// Fetches the user's most played tracks from ListenBrainz and plays those in the library.
    fn play_listenbrainz_favorites(self: &Rc<Self>) {
        let (token, user) = {
            let state = self.state.borrow();
            let scrobble::Connection::Connected(user, _) = &*self.listenbrainz_state.borrow()
            else {
                return;
            };
            (state.listenbrainz_token.trim().to_owned(), user.clone())
        };
        // Tags already read are reused; the rest are read in the background with the request.
        let tracks: Vec<(PathBuf, Option<(String, String)>)> = {
            let state = self.state.borrow();
            let info = self.info.borrow();
            state
                .tracks
                .iter()
                .filter(|track| state.library_track_visible(track))
                .map(|track| {
                    let known = info
                        .get(track)
                        .and_then(|info| Some((info.title.clone()?, info.artist.clone()?)));
                    (track.clone(), known)
                })
                .collect()
        };
        self.message(self.text(
            "Looking up your most played tracks on ListenBrainz…",
            "Buscando suas mais ouvidas no ListenBrainz…",
        ));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || {
                let Some(recordings) = listenbrainz::top_recordings(&token, &user)? else {
                    return Ok(None);
                };
                let library: Vec<(PathBuf, String, String)> = tracks
                    .into_iter()
                    .filter_map(|(path, known)| {
                        let (title, artist) = known.or_else(|| {
                            let info = read_track(&path);
                            Some((info.title?, info.artist?))
                        })?;
                        Some((path, title, artist))
                    })
                    .collect();
                Ok::<_, listenbrainz::Error>(Some(match_recordings(&recordings, &library)))
            })
            .await
            .unwrap_or(Err(listenbrainz::Error::Unavailable));
            let Some(player) = weak.upgrade() else { return };
            match result {
                Ok(None) => player.message(player.text(
                    "ListenBrainz has not calculated your statistics yet. It does so once a day, so try again tomorrow.",
                    "O ListenBrainz ainda não calculou suas estatísticas. Ele faz isso uma vez por dia, então tente de novo amanhã.",
                )),
                Ok(Some(tracks)) if !tracks.is_empty() => {
                    player.message("");
                    player.replace_queue(tracks, 0);
                    player.show(Page::Queue);
                }
                Ok(Some(_)) => player.message(player.text(
                    "None of your most played tracks on ListenBrainz are in the library yet.",
                    "Nenhuma das suas mais ouvidas no ListenBrainz está na biblioteca ainda.",
                )),
                Err(_) => player.message(player.text(
                    "ListenBrainz could not be reached. Check the connection and that curl is installed.",
                    "Não foi possível falar com o ListenBrainz. Confira a conexão e se o curl está instalado.",
                )),
            }
        });
    }
}

/// Library tracks for the recordings, in the recordings' order, matching titles without
/// version notes and any shared credited artist.
fn match_recordings(
    recordings: &[listenbrainz::Recording],
    library: &[(PathBuf, String, String)],
) -> Vec<PathBuf> {
    let key = |title: &str| lyrics::fold_search_name(&lyrics::core_title(title));
    let mut found = Vec::new();
    for recording in recordings {
        let title = key(&recording.title);
        if title.is_empty() {
            continue;
        }
        if let Some((path, _, _)) = library.iter().find(|(path, local_title, local_artist)| {
            key(local_title) == title
                && lyrics::artists_match(&recording.artist, local_artist)
                && !found.contains(path)
        }) {
            found.push(path.clone());
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listenbrainz_favorites_match_library_tracks_in_order() {
        let recording = |artist: &str, title: &str| listenbrainz::Recording {
            artist: artist.into(),
            title: title.into(),
        };
        let library = vec![
            (
                PathBuf::from("a"),
                "Lucro (Descomprimindo)".into(),
                "BaianaSystem".into(),
            ),
            (PathBuf::from("b"), "Clandestino".into(), "Manu Chao".into()),
            (
                PathBuf::from("c"),
                "Come As You Are (Remastered)".into(),
                "Nirvana".into(),
            ),
        ];
        let found = match_recordings(
            &[
                recording("Nirvana", "Come as You Are"),
                recording("Someone Else", "Clandestino"),
                recording("Manu Chao", "Clandestino"),
                recording("Manu Chao", "Clandestino"),
                recording("Absent", "Not Here"),
            ],
            &library,
        );
        assert_eq!(found, [PathBuf::from("c"), PathBuf::from("b")]);
    }
}
