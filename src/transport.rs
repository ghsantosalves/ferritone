//! Starting, stopping, and advancing playback, and the playback bar.

use super::*;

impl Player {
    /// Keeps the window from getting smaller than the page, the playback bar and the header
    /// need. libadwaita lets a window with breakpoints shrink past its content, which then gets
    /// cut off on the right or at the bottom. For the width, the seek bar is left out because it
    /// hides on its own in narrow windows, and the track table drops columns as the page
    /// narrows, so the minimum settles at the narrow layout. Track lists scroll, so the height
    /// keeps the header, the playback bar and the rest of the page's minimum in view.
    pub(crate) fn fit_window_minimum(&self) {
        let minimum = |widget: &gtk::Widget| widget.measure(gtk::Orientation::Horizontal, -1).0;
        let mut transport = minimum(self.transport.upcast_ref());
        if self.progress.get_visible() {
            transport -= minimum(self.progress.upcast_ref()) + self.transport.spacing();
        }
        let header = self
            .window
            .content()
            .and_downcast::<adw::ToolbarView>()
            .and_then(|toolbar| toolbar.first_child())
            .map_or(0, |bars| minimum(&bars));
        let needed = [minimum(self.content.upcast_ref()), transport, header]
            .into_iter()
            .max()
            .unwrap_or(0);
        if needed > 0 && self.window.width_request() != needed {
            self.window.set_width_request(needed);
        }
        let Some(layout) = self.window.content() else {
            return;
        };
        let width = self.window.width().max(needed);
        let height = layout.measure(gtk::Orientation::Vertical, width).0;
        if height > 0 && self.window.height_request() != height {
            self.window.set_height_request(height);
        }
    }

    pub(crate) fn fit_transport(&self) {
        let view = self.state.borrow().view;
        let height = view.transport_height.clamp(0, 130);
        let max_width = (self.window.width() / 3).clamp(95, 350);
        let lines = ((height - 9).max(36) / 16).clamp(2, 5) - 1;
        if self.now_playing.lines() != lines {
            self.now_playing.set_lines(lines);
        }
        let thumbnail = (height - 12).clamp(36, if self.window.width() > 760 { 92 } else { 60 });
        if self.thumbnail_size.replace(thumbnail) != thumbnail {
            self.update_thumbnail(thumbnail);
        }
        let width = if view.transport_title_width > 0 {
            view.transport_title_width.clamp(80, max_width)
        } else {
            0
        };
        if self.transport.height_request() != height {
            self.transport.set_size_request(-1, height);
        }
        if self.track_caption.width_request() != width {
            self.track_caption.set_size_request(width, -1);
        }
    }

    pub(crate) fn seek(&self, seconds: f64) {
        if !self.updating_progress.get() && self.active.get() {
            let target = gst::ClockTime::from_mseconds((seconds.max(0.0) * 1000.0) as u64);
            let sought = self
                .pipeline
                .seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT, target);
            if sought.is_ok() {
                self.pending_seek_signal.set(true);
            }
        }
    }

    pub(crate) fn play_library_track(self: &Rc<Self>, track: &std::path::Path) {
        let state = self.state.borrow();
        if state.shuffle {
            let queue: Vec<_> = state
                .tracks
                .iter()
                .filter(|entry| state.library_track_visible(entry))
                .cloned()
                .collect();
            if let Some(index) = queue.iter().position(|entry| entry == track) {
                drop(state);
                self.replace_queue(queue, index);
                return;
            }
        }
        drop(state);
        self.replace_queue(vec![track.to_owned()], 0);
    }

    pub(crate) fn replace_queue(self: &Rc<Self>, tracks: Vec<PathBuf>, index: usize) {
        self.replace_queue_from(tracks, index, None);
    }

    pub(crate) fn replace_queue_from(
        self: &Rc<Self>,
        tracks: Vec<PathBuf>,
        index: usize,
        playlist: Option<usize>,
    ) {
        if tracks.is_empty() {
            return;
        }
        {
            let mut state = self.state.borrow_mut();
            state.queue = tracks;
            state.queue_playlist = playlist;
            state.current = index;
            state.reset_shuffle_order();
        }
        self.start(index, 0);
        if self.page.get() == Page::Queue {
            self.show(Page::Queue);
        }
    }

    pub(crate) fn start(self: &Rc<Self>, index: usize, position_ms: u64) {
        let track = self.state.borrow().queue.get(index).cloned();
        let Some(track) = track else { return };
        self.message("");
        self.failures.borrow_mut().started();
        self.play_counted.set(false);
        self.track_started();
        self.cancel_handoff();
        let _ = self.pipeline.set_state(gst::State::Null);
        if let Some(waveform) = &self.waveform {
            waveform.clear();
        }
        self.pipeline
            .set_property("uri", gio::File::for_path(&track).uri().as_str());
        self.pending_seek.set(position_ms);
        self.pending_seek_signal.set(false);
        self.state.borrow_mut().current = index;
        self.state.borrow_mut().position_ms = position_ms;
        let started = self.pipeline.set_state(gst::State::Playing).is_ok();
        self.active.set(started);
        self.playing.set(started);
        if !started {
            self.message(self.text(
                "Could not start playback.",
                "Não foi possível iniciar a reprodução.",
            ));
        }
        self.load_current_track();
        self.update_transport();
        self.refresh_track_highlight();
        self.refresh_playing_playlist();
        self.save();
    }

    pub(crate) fn toggle(self: &Rc<Self>) {
        if self.active.get() {
            let play = !self.playing.get();
            let state = if play {
                gst::State::Playing
            } else {
                gst::State::Paused
            };
            if self.pipeline.set_state(state).is_ok() {
                self.playing.set(play);
                self.update_position();
                self.update_transport();
                self.save();
            }
        } else {
            let (current, position) = {
                let state = self.state.borrow();
                (state.current, state.position_ms)
            };
            self.start(current, position);
        }
    }

    pub(crate) fn step(self: &Rc<Self>, offset: isize) {
        let index = self.state.borrow().next_index(offset, false);
        if let Some(index) = index {
            self.start(index, 0);
        }
    }

    pub(crate) fn update_position(&self) {
        if !self.active.get() || self.pending_seek.get() != 0 {
            return;
        }
        if let Some(position) = self.pipeline.query_position::<gst::ClockTime>() {
            let position = position.mseconds();
            self.state.borrow_mut().position_ms = position;
            self.failures.borrow_mut().progressed(position);
        }
    }

    pub(crate) fn update_transport(&self) {
        let (track, position, has_queue) = {
            let state = self.state.borrow();
            (
                state.queue.get(state.current).cloned(),
                state.position_ms,
                !state.queue.is_empty(),
            )
        };
        let info = self.info.borrow();
        let metadata = track.as_ref().and_then(|path| info.get(path));
        let name = match track.as_ref() {
            Some(track) => metadata
                .and_then(|info| info.title.clone())
                .unwrap_or_else(|| title(track)),
            None => self.text("Nothing playing", "Nenhuma faixa").to_owned(),
        };
        let artist = metadata
            .and_then(|info| info.artist.as_deref())
            .unwrap_or("");
        if self.now_playing.text() != name {
            self.now_playing.set_text(&name);
            self.now_playing.set_tooltip_text(Some(&name));
        }
        if self.playing_artist.text() != artist {
            self.playing_artist.set_text(artist);
            self.playing_artist
                .set_tooltip_text((!artist.is_empty()).then_some(artist));
        }
        set_visible(&self.playing_artist, !artist.is_empty());
        if self.toggle.is_sensitive() != has_queue {
            self.toggle.set_sensitive(has_queue);
        }
        let (icon, hint) = if self.playing.get() {
            (
                "media-playback-pause-symbolic",
                self.text("Pause", "Pausar"),
            )
        } else {
            ("media-playback-start-symbolic", self.text("Play", "Tocar"))
        };
        if self.toggle.icon_name().as_deref() != Some(icon) {
            self.toggle.set_icon_name(icon);
        }
        if self.toggle.tooltip_text().as_deref() != Some(hint) {
            self.toggle.set_tooltip_text(Some(hint));
        }
        let duration = self
            .pipeline
            .query_duration::<gst::ClockTime>()
            .map_or(0, |value| value.mseconds());
        self.updating_progress.set(true);
        let upper = (duration as f64 / 1000.0).max(1.0);
        if self.progress.adjustment().upper() != upper {
            self.progress.set_range(0.0, upper);
        }
        if self.progress.get_sensitive() != (duration > 0) {
            self.progress.set_sensitive(duration > 0);
        }
        let value = if duration > 0 {
            position as f64 / 1000.0
        } else {
            0.0
        };
        if self.progress.value() != value {
            self.progress.set_value(value);
        }
        self.updating_progress.set(false);
        let display_duration = if duration > 0 {
            duration
        } else {
            metadata.and_then(|info| info.duration_ms).unwrap_or(0)
        };
        let total = timestamp(display_duration);
        let width = (timestamp(position.max(display_duration)).len() + 3 + total.len()) as i32;
        if self.time.width_chars() != width {
            self.time.set_width_chars(width);
        }
        let time = format!("{} / {total}", timestamp(position));
        if self.time.text() != time {
            self.time.set_text(&time);
        }
        self.update_lyrics();
        if let Some(session) = self.media_session.borrow().as_ref() {
            session.publish(self);
        }
    }

    pub(crate) fn update_repeat(&self) {
        let (icon, hint, enabled) = match self.state.borrow().repeat {
            Repeat::Off => (
                "media-playlist-repeat-symbolic",
                self.text("Repeat off", "Repetição desativada"),
                false,
            ),
            Repeat::Queue => (
                "media-playlist-repeat-symbolic",
                self.text("Repeat queue", "Repetir fila"),
                true,
            ),
            Repeat::Track => (
                "media-playlist-repeat-song-symbolic",
                self.text("Repeat track", "Repetir faixa"),
                true,
            ),
        };
        self.repeat.set_icon_name(icon);
        self.repeat.set_tooltip_text(Some(hint));
        self.repeat
            .update_property(&[gtk::accessible::Property::Label(hint)]);
        if enabled {
            self.repeat.add_css_class("enabled");
        } else {
            self.repeat.remove_css_class("enabled");
        }
    }

    pub(crate) fn poll(self: &Rc<Self>) {
        if let Some(bus) = self.pipeline.bus() {
            while let Some(message) = bus.pop() {
                match message.view() {
                    gst::MessageView::Eos(..)
                        if self.sleep.get() == playback::SleepTimer::EndOfTrack =>
                    {
                        self.sleep_at_track_end();
                    }
                    gst::MessageView::Eos(..) => {
                        let index = self.state.borrow().next_index(1, true);
                        if let Some(index) = index {
                            self.start(index, 0);
                        } else {
                            self.stop();
                        }
                    }
                    gst::MessageView::Error(error) => {
                        eprintln!("Playback error: {}", error.error());
                        // A track queued without a gap may be the one that failed.
                        self.follow_handoff();
                        let decision = {
                            let state = self.state.borrow();
                            let has_next = state.next_index(1, false).is_some();
                            self.failures.borrow_mut().failed(
                                state.current,
                                state.queue.len(),
                                has_next,
                            )
                        };
                        match decision {
                            playback::OnError::Skip => {
                                let count = self.failures.borrow().count();
                                let next = self.state.borrow().next_index(1, false);
                                if let Some(index) = next {
                                    self.start(index, 0);
                                }
                                self.failures.borrow_mut().skipped(count);
                                self.message(self.text(
                                    "Skipped a track that could not be played.",
                                    "Uma faixa que não pôde ser tocada foi pulada.",
                                ));
                            }
                            playback::OnError::GiveUp { resume, several } => {
                                if several {
                                    self.state.borrow_mut().current = resume;
                                    self.load_current_track();
                                    self.refresh_track_highlight();
                                }
                                self.stop();
                                self.update_transport();
                                self.message(if several {
                                    self.text(
                                        "Several tracks in a row could not be played. Check your audio output.",
                                        "Várias faixas seguidas não puderam ser tocadas. Verifique a saída de áudio.",
                                    )
                                } else {
                                    self.text(
                                        "This track could not be played.",
                                        "Não foi possível tocar esta faixa.",
                                    )
                                });
                            }
                        }
                        // Later messages belong to the failed pipeline run.
                        break;
                    }
                    gst::MessageView::StreamStart(..) => self.follow_handoff(),
                    gst::MessageView::AsyncDone(..) => {
                        let position = self.pending_seek.replace(0);
                        if position > 0 {
                            self.seek(position as f64 / 1000.0);
                        } else if self.pending_seek_signal.replace(false) {
                            if let (Some(session), Some(position)) = (
                                self.media_session.borrow().as_ref(),
                                self.pipeline.query_position::<gst::ClockTime>(),
                            ) {
                                session.seeked(position.useconds() as i64);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        self.plan_handoff();
        self.poll_remote();
        self.poll_sleep();
        self.update_inhibit();
        self.count_play();
        self.update_position();
        self.update_transport();
        // Keep the resume point recent in case the session ends without closing the window.
        if self
            .autosave
            .borrow()
            .due(Instant::now(), self.playing.get())
        {
            self.save_position();
        }
        if self.window.is_mapped() {
            self.fit_transport();
            self.fit_search();
            self.fit_columns();
            self.fit_window_minimum();
        }
    }

    pub(crate) fn plan_handoff(&self) {
        // Stopping at the end of the track means nothing follows it.
        let stop_after = self.sleep.get() == playback::SleepTimer::EndOfTrack;
        let next = if self.active.get() && !stop_after {
            let state = self.state.borrow();
            state
                .next_index(1, true)
                .and_then(|index| Some((index, state.queue.get(index)?.clone())))
        } else {
            None
        };
        if let Ok(mut handoff) = self.handoff.lock() {
            handoff.plan(next);
        }
    }

    pub(crate) fn cancel_handoff(&self) {
        if let Ok(mut handoff) = self.handoff.lock() {
            handoff.cancel();
        }
    }

    /// Catches up with a track that GStreamer moved to on its own.
    pub(crate) fn follow_handoff(self: &Rc<Self>) {
        let switched = self
            .handoff
            .lock()
            .ok()
            .and_then(|mut handoff| handoff.switched());
        let Some((index, track)) = switched else {
            return;
        };
        let index = playback::resolve(&self.state.borrow().queue, index, &track);
        let Some(index) = index else { return };
        {
            let mut state = self.state.borrow_mut();
            state.current = index;
            state.position_ms = 0;
        }
        self.pending_seek.set(0);
        self.pending_seek_signal.set(false);
        self.failures.borrow_mut().started();
        self.play_counted.set(false);
        self.load_current_track();
        self.track_started();
        self.update_transport();
        self.refresh_track_highlight();
        self.refresh_playing_playlist();
        self.save();
    }

    pub(crate) fn stop(&self) {
        self.cancel_handoff();
        let _ = self.pipeline.set_state(gst::State::Null);
        self.active.set(false);
        self.playing.set(false);
        if let Some(waveform) = &self.waveform {
            waveform.clear();
        }
        self.pending_seek.set(0);
        self.pending_seek_signal.set(false);
        self.state.borrow_mut().position_ms = 0;
        self.refresh_track_highlight();
        self.refresh_playing_playlist();
        self.save();
    }
}
