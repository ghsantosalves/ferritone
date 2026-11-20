//! Track metadata, artwork, and lyrics for the current track.

use super::*;

impl Player {
    pub(crate) fn scan_metadata(self: &Rc<Self>, tracks: &[PathBuf]) {
        let known = self.info.borrow();
        let mut reading = self.reading_info.borrow_mut();
        let missing: Vec<_> = tracks
            .iter()
            .filter(|path| !known.contains_key(*path) && reading.insert((*path).clone()))
            .cloned()
            .collect();
        drop(known);
        drop(reading);
        if missing.is_empty() {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || {
                missing
                    .into_iter()
                    .map(|path| {
                        let info = read_track(&path);
                        (path, info)
                    })
                    .collect::<Vec<_>>()
            })
            .await;
            let (Some(player), Ok(items)) = (weak.upgrade(), result) else {
                return;
            };
            {
                let mut known = player.info.borrow_mut();
                let mut reading = player.reading_info.borrow_mut();
                for (path, info) in items {
                    reading.remove(&path);
                    known.insert(path, info);
                }
            }
            let page = player.page.get();
            if page == Page::Library {
                if let Some(refresh) = player.library_refresh.borrow().as_ref() {
                    refresh(&player.library_search.borrow());
                }
            } else if matches!(
                page,
                Page::Queue
                    | Page::Playlist(_)
                    | Page::Albums
                    | Page::Album
                    | Page::Artists
                    | Page::Artist
            ) {
                player.show(page);
            }
            player.update_transport();
        });
    }

    pub(crate) fn load_current_track(self: &Rc<Self>) {
        let path = {
            let state = self.state.borrow();
            state.queue.get(state.current).cloned()
        };
        if *self.artwork_track.borrow() == path {
            return;
        }
        self.artwork_track.replace(path.clone());
        self.refresh_lyrics();
        self.artwork_button.set_sensitive(false);
        self.cover.replace(None);
        self.cover_source.set_paintable(gdk::Paintable::NONE);
        self.thumbnail.replace(None);
        self.artwork.set_icon_name(Some("audio-x-generic-symbolic"));
        let Some(path) = path else { return };
        self.scan_metadata(std::slice::from_ref(&path));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let file = path.clone();
            let result = gio::spawn_blocking(move || read_cover_image(&file)).await;
            let (Some(player), Ok(Some((original, thumbnail)))) = (weak.upgrade(), result) else {
                return;
            };
            if player.artwork_track.borrow().as_ref() != Some(&path) {
                return;
            }
            player
                .thumbnail
                .replace(thumbnail.map(|image| image.pixbuf()));
            player.update_thumbnail(player.thumbnail_size.get());
            let cover = gdk::Texture::for_pixbuf(&original.pixbuf());
            player.cover_source.set_paintable(Some(&cover));
            player.cover.replace(Some(cover));
            player.artwork_button.set_sensitive(true);
        });
    }

    pub(crate) fn set_lyrics_enabled(self: &Rc<Self>, enabled: bool) {
        if self.state.borrow().view.lyrics == enabled {
            return;
        }
        self.state.borrow_mut().view.lyrics = enabled;
        if let Some(button) = self.lyrics_preference.upgrade() {
            button.set_active(enabled);
        }
        self.refresh_lyrics();
        self.save();
    }

    pub(crate) fn set_online_lyrics_enabled(self: &Rc<Self>, enabled: bool) {
        if self.state.borrow().view.online_lyrics == enabled {
            return;
        }
        self.state.borrow_mut().view.online_lyrics = enabled;
        if let Some(button) = self.online_lyrics_preference.upgrade() {
            button.set_active(enabled);
        }
        self.refresh_lyrics();
        self.save();
    }

    pub(crate) fn refresh_lyrics(self: &Rc<Self>) {
        use lyrics::Lyrics;
        self.lyric_drag_preview.set(None);
        let request = self.lyric_request.get().wrapping_add(1);
        self.lyric_request.set(request);
        self.lyrics.replace(Lyrics::Missing);
        self.update_lyrics();
        let view = self.state.borrow().view;
        if !view.lyrics {
            return;
        }
        let Some(path) = self.artwork_track.borrow().clone() else {
            return;
        };
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let file = path.clone();
            let local = gio::spawn_blocking(move || Lyrics::read(&file))
                .await
                .unwrap_or_default();
            let Some(player) = weak
                .upgrade()
                .filter(|player| player.lyric_request.get() == request)
            else {
                return;
            };
            if !matches!(local, Lyrics::Missing) || !view.online_lyrics {
                player.lyrics.replace(local);
                player.update_lyrics();
                return;
            }
            if let Some(cached) = player.online_lyrics_cache.borrow().get(&path).cloned() {
                player.lyrics.replace(cached);
                player.update_lyrics();
                return;
            }
            drop(player);
            glib::timeout_future(Duration::from_millis(400)).await;
            if weak
                .upgrade()
                .is_none_or(|player| player.lyric_request.get() != request)
            {
                return;
            }
            let file = path.clone();
            let online =
                gio::spawn_blocking(move || Lyrics::lookup_online(&read_track(&file), &file))
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();
            let Some(player) = weak
                .upgrade()
                .filter(|player| player.lyric_request.get() == request)
            else {
                return;
            };
            {
                let mut cache = player.online_lyrics_cache.borrow_mut();
                if cache.len() >= 64 {
                    cache.clear();
                }
                cache.insert(path, online.clone());
            }
            player.lyrics.replace(online);
            player.update_lyrics();
        });
    }

    pub(crate) fn update_lyrics(&self) {
        let state = self.state.borrow();
        let lyrics = self.lyrics.borrow();
        let synced = matches!(*lyrics, lyrics::Lyrics::Timed(_));
        let mini = match &*lyrics {
            lyrics::Lyrics::Plain(text) => text.as_str(),
            lyrics::Lyrics::Timed(_) => "",
            lyrics::Lyrics::Missing => self.text(
                "No lyrics available for this track.",
                "Nenhuma letra disponível para esta faixa.",
            ),
        };
        if self.mini_lyrics.text() != mini {
            self.mini_lyrics.set_text(mini);
        }
        set_visible(&self.mini_lyrics, !synced);
        let timing = state
            .queue
            .get(state.current)
            .and_then(|track| state.lyric_offsets_ms.get(track))
            .copied()
            .unwrap_or(0);
        let excerpt = match self.lyric_drag_preview.get() {
            Some(preview) => lyrics.excerpt(preview, 0),
            None => lyrics.excerpt(state.position_ms, timing),
        };
        let has_verse = excerpt[2].is_some_and(|line| !line.is_empty());
        let center = if excerpt[2].is_none() {
            excerpt[3].filter(|line| !line.is_empty()).unwrap_or("♪")
        } else {
            "♪"
        };
        let opacity = if has_verse { 1.0 } else { 0.62 };
        if (self.lyric_lines[2].opacity() - opacity).abs() > 0.01 {
            self.lyric_lines[2].set_opacity(opacity);
        }
        let height = state.view.lyrics_height.clamp(0, 130);
        if self.lyric_strip.height_request() != height {
            self.lyric_strip.set_height_request(height);
        }
        let show_neighbors = self.window.width() >= 760 && matches!(height, 0 | 52..);
        for (index, (widget, text)) in self.lyric_lines.iter().zip(excerpt).enumerate() {
            let text = if synced && !has_verse {
                if index == 2 { center } else { " " }
            } else {
                text.filter(|line| !line.is_empty()).unwrap_or(" ")
            };
            if widget.text() != text {
                widget.set_text(text);
            }
            if matches!(index, 1 | 3) {
                set_visible(widget, show_neighbors);
            }
        }
        set_visible(
            &self.lyric_panel,
            state.view.lyrics && self.active.get() && synced,
        );
    }

    pub(crate) fn show_cover(&self) {
        let cover = self.cover.borrow();
        let Some(cover) = cover.as_ref() else { return };
        let dialog = gtk::Window::builder()
            .transient_for(&self.window)
            .destroy_with_parent(true)
            .default_width(520)
            .default_height(520)
            .resizable(true)
            .title(self.text("Album cover", "Capa do álbum"))
            .build();
        let picture = gtk::Picture::for_paintable(cover);
        picture.set_can_shrink(true);
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_alternative_text(Some(self.text("Album cover", "Capa do álbum")));
        dialog.set_child(Some(&picture));
        dialog.present();
    }

    pub(crate) fn update_thumbnail(&self, size: i32) {
        self.artwork.set_size_request(size, size);
        self.artwork.set_pixel_size(size);
        let source = self.thumbnail.borrow();
        if let Some(pixbuf) = source.as_ref().and_then(|source| scale_image(source, size)) {
            self.artwork
                .set_paintable(Some(&gdk::Texture::for_pixbuf(&pixbuf)));
        }
    }
}
