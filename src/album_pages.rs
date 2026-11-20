//! The Albums and Artists pages, an album's page, and album cover thumbnails.

use super::*;
use albums::{Album, AlbumKey};

/// Cover thumbnails are drawn at this size; they are decoded at twice it for sharp HiDPI.
const COVER_SIZE: i32 = 150;

impl Player {
    /// Albums of the visible library tracks; asks for any tags that have not been read yet.
    fn library_albums(self: &Rc<Self>) -> Vec<Album> {
        let tracks: Vec<PathBuf> = {
            let state = self.state.borrow();
            state
                .tracks
                .iter()
                .filter(|track| state.library_track_visible(track))
                .cloned()
                .collect()
        };
        self.scan_metadata(&tracks);
        albums::group_albums(&tracks, &self.info.borrow(), library_collation_key)
    }

    fn unknown_artist(&self, name: &str) -> String {
        if name.is_empty() {
            self.text("Unknown artist", "Artista desconhecido")
                .to_owned()
        } else {
            name.to_owned()
        }
    }

    fn unknown_album(&self, name: &str) -> String {
        if name.is_empty() {
            self.text("Unknown album", "Álbum desconhecido").to_owned()
        } else {
            name.to_owned()
        }
    }

    fn library_is_empty_hint(self: &Rc<Self>) -> bool {
        if !self.state.borrow().tracks.is_empty() {
            return false;
        }
        self.hint(self.text(
            "Add a folder to the library to browse it by album and artist.",
            "Adicione uma pasta à biblioteca para navegar por álbum e artista.",
        ));
        true
    }

    pub(crate) fn albums_page(self: &Rc<Self>) {
        self.heading(self.text("Albums", "Álbuns"));
        if self.library_is_empty_hint() {
            return;
        }
        let albums = self.library_albums();
        self.content.append(&self.album_grid(&albums));
    }

    pub(crate) fn artists_page(self: &Rc<Self>) {
        self.heading(self.text("Artists", "Artistas"));
        if self.library_is_empty_hint() {
            return;
        }
        let artists = albums::group_artists(&self.library_albums());
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        for (position, artist) in artists.iter().enumerate() {
            let key = artist.key.clone();
            let item = button("", self, move |player| {
                player.selected_artist.replace(key.clone());
                player.show(Page::Artist);
            });
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            line.append(&gtk::Image::from_icon_name("avatar-default-symbolic"));
            let name = table_label(&self.unknown_artist(&artist.name), 0);
            name.set_hexpand(true);
            line.append(&name);
            let albums = if artist.albums == 1 {
                self.text("1 album", "1 álbum").to_owned()
            } else {
                self.text("{} albums", "{} álbuns")
                    .replace("{}", &artist.albums.to_string())
            };
            let tracks = if artist.tracks == 1 {
                self.text("1 track", "1 faixa").to_owned()
            } else {
                self.text("{} tracks", "{} faixas")
                    .replace("{}", &artist.tracks.to_string())
            };
            let counts = gtk::Label::new(Some(&format!("{albums} · {tracks}")));
            counts.add_css_class("dim-label");
            line.append(&counts);
            item.set_child(Some(&line));
            item.set_hexpand(true);
            item.add_css_class("track-title");
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            style_row(&row, position);
            row.append(&item);
            rows.append(&row);
        }
        let scroll = gtk::ScrolledWindow::builder()
            .child(&rows)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        self.content.append(&scroll);
    }

    pub(crate) fn artist_page(self: &Rc<Self>) {
        let key = self.selected_artist.borrow().clone();
        let albums: Vec<Album> = self
            .library_albums()
            .into_iter()
            .filter(|album| album.key.artist == key)
            .collect();
        let name = albums
            .first()
            .map(|album| album.artist.clone())
            .unwrap_or_default();
        self.heading(&self.unknown_artist(&name));
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.add_css_class("page-actions");
        let tracks: Vec<PathBuf> = albums
            .iter()
            .flat_map(|album| album.tracks.clone())
            .collect();
        let play = button(self.text("Play", "Tocar"), self, move |player| {
            player.replace_queue(tracks.clone(), 0);
        });
        play.add_css_class("suggested-action");
        actions.append(&play);
        actions.append(&button(
            self.text("All artists", "Todos os artistas"),
            self,
            |player| player.show(Page::Artists),
        ));
        self.content.append(&actions);
        self.content.append(&self.album_grid(&albums));
    }

    fn album_grid(self: &Rc<Self>, albums: &[Album]) -> gtk::ScrolledWindow {
        let grid = gtk::FlowBox::builder()
            .homogeneous(true)
            .selection_mode(gtk::SelectionMode::None)
            .min_children_per_line(2)
            .max_children_per_line(12)
            .column_spacing(12)
            .row_spacing(16)
            .valign(gtk::Align::Start)
            .build();
        for album in albums {
            let key = album.key.clone();
            let card = button("", self, move |player| {
                player.selected_album.replace(key.clone());
                player.show(Page::Album);
            });
            card.add_css_class("flat");
            card.add_css_class("album-card");
            let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
            let cover = gtk::Image::from_icon_name("media-optical-symbolic");
            cover.set_pixel_size(COVER_SIZE);
            cover.add_css_class("album-cover");
            cover.set_overflow(gtk::Overflow::Hidden);
            if let Some(first) = album.tracks.first() {
                self.load_album_cover(&cover, first);
            }
            content.append(&cover);
            let title = gtk::Label::new(Some(&self.unknown_album(&album.title)));
            title.set_ellipsize(gtk::pango::EllipsizeMode::End);
            title.set_max_width_chars(16);
            title.add_css_class("album-title");
            content.append(&title);
            let artist = gtk::Label::new(Some(&self.unknown_artist(&album.artist)));
            artist.set_ellipsize(gtk::pango::EllipsizeMode::End);
            artist.set_max_width_chars(16);
            artist.add_css_class("dim-label");
            content.append(&artist);
            card.set_child(Some(&content));
            card.set_tooltip_text(Some(&format!(
                "{} · {}",
                self.unknown_album(&album.title),
                self.unknown_artist(&album.artist)
            )));
            grid.append(&card);
        }
        gtk::ScrolledWindow::builder()
            .child(&grid)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build()
    }

    pub(crate) fn album_page(self: &Rc<Self>) {
        let key: AlbumKey = self.selected_album.borrow().clone();
        let Some(album) = self
            .library_albums()
            .into_iter()
            .find(|album| album.key == key)
        else {
            self.heading(self.text("Albums", "Álbuns"));
            self.hint(self.text(
                "This album is no longer in the library.",
                "Este álbum não está mais na biblioteca.",
            ));
            return;
        };
        self.heading(&self.unknown_album(&album.title));
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        let cover = gtk::Image::from_icon_name("media-optical-symbolic");
        cover.set_pixel_size(COVER_SIZE);
        cover.add_css_class("album-cover");
        cover.set_overflow(gtk::Overflow::Hidden);
        if let Some(first) = album.tracks.first() {
            self.load_album_cover(&cover, first);
        }
        header.append(&cover);
        let details = gtk::Box::new(gtk::Orientation::Vertical, 8);
        details.set_valign(gtk::Align::Center);
        let artist_key = album.key.artist.clone();
        let artist = button(&self.unknown_artist(&album.artist), self, move |player| {
            player.selected_artist.replace(artist_key.clone());
            player.show(Page::Artist);
        });
        artist.add_css_class("flat");
        artist.set_halign(gtk::Align::Start);
        artist.set_tooltip_text(Some(self.text("Show the artist", "Mostrar o artista")));
        details.append(&artist);
        let info = self.info.borrow();
        let total: u64 = album
            .tracks
            .iter()
            .filter_map(|track| info.get(track)?.duration_ms)
            .sum();
        let count = if album.tracks.len() == 1 {
            self.text("1 track", "1 faixa").to_owned()
        } else {
            self.text("{} tracks", "{} faixas")
                .replace("{}", &album.tracks.len().to_string())
        };
        let summary = gtk::Label::new(Some(&format!("{count} · {}", timestamp(total))));
        summary.set_xalign(0.0);
        summary.add_css_class("dim-label");
        details.append(&summary);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.add_css_class("page-actions");
        let tracks = album.tracks.clone();
        let play = button(self.text("Play", "Tocar"), self, move |player| {
            player.replace_queue(tracks.clone(), 0);
        });
        play.add_css_class("suggested-action");
        actions.append(&play);
        let tracks = album.tracks.clone();
        actions.append(&button(
            self.text("Add to queue", "Adicionar à fila"),
            self,
            move |player| {
                player
                    .state
                    .borrow_mut()
                    .insert_queue_tracks(None, tracks.clone());
                player.save();
                player.update_transport();
                player.message(player.text("Added to the queue.", "Adicionado à fila."));
            },
        ));
        details.append(&actions);
        header.append(&details);
        self.content.append(&header);

        let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let current = {
            let state = self.state.borrow();
            state.queue.get(state.current).cloned()
        };
        for (index, track) in album.tracks.iter().enumerate() {
            let metadata = info.get(track);
            let tracks = album.tracks.clone();
            let item = button("", self, move |player| {
                player.replace_queue(tracks.clone(), index);
            });
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let number = gtk::Label::new(Some(
                &metadata
                    .and_then(|info| info.track_number)
                    .map_or_else(|| (index + 1).to_string(), |number| number.to_string()),
            ));
            number.set_width_chars(3);
            number.set_xalign(1.0);
            number.add_css_class("dim-label");
            line.append(&number);
            let name = table_label(
                &metadata
                    .and_then(|info| info.title.clone())
                    .unwrap_or_else(|| title(track)),
                0,
            );
            name.set_hexpand(true);
            line.append(&name);
            // Show the track artist where it differs from the album's, as on compilations.
            let track_artist = metadata
                .and_then(|info| info.artist.as_deref())
                .unwrap_or("");
            if !track_artist.is_empty() && track_artist.to_lowercase() != album.key.artist {
                let label = table_label(track_artist, 0);
                label.add_css_class("dim-label");
                line.append(&label);
            }
            line.append(&gtk::Label::new(Some(&timestamp(
                metadata.and_then(|info| info.duration_ms).unwrap_or(0),
            ))));
            item.set_child(Some(&line));
            item.set_hexpand(true);
            item.add_css_class("track-title");
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            style_row(&row, index);
            if current.as_ref() == Some(track) {
                row.add_css_class("playing");
            }
            row.append(&item);
            rows.append(&row);
        }
        drop(info);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&rows)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        self.content.append(&scroll);
    }

    /// Shows an album's cover in `image`, reading it in the background the first time.
    fn load_album_cover(self: &Rc<Self>, image: &gtk::Image, track: &PathBuf) {
        if let Some(cached) = self.album_covers.borrow().get(track) {
            if let Some(texture) = cached {
                image.set_paintable(Some(texture));
            }
            return;
        }
        let weak_image = image.downgrade();
        let weak = Rc::downgrade(self);
        let track = track.clone();
        glib::spawn_future_local(async move {
            let file = track.clone();
            let side = COVER_SIZE * 2;
            let image = gio::spawn_blocking(move || read_cover_thumbnail(&file, side))
                .await
                .ok()
                .flatten();
            let Some(player) = weak.upgrade() else { return };
            let texture = image.map(|image| gdk::Texture::for_pixbuf(&image.pixbuf()));
            player
                .album_covers
                .borrow_mut()
                .insert(track, texture.clone());
            if let (Some(image), Some(texture)) = (weak_image.upgrade(), texture) {
                image.set_paintable(Some(&texture));
            }
        });
    }
}
