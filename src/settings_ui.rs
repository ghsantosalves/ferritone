//! Equalizer, View, and Preferences windows, and theming.

use super::*;

impl Player {
    pub(crate) fn show_equalizer(self: &Rc<Self>) {
        if self.equalizer.is_none() {
            gtk::AlertDialog::builder()
                .message(self.text("Equalizer unavailable", "Equalizador indisponível"))
                .detail(self.text(
                    "Install the GStreamer Good Plug-ins to use the equalizer.",
                    "Instale o pacote de plugins Good do GStreamer para usar o equalizador.",
                ))
                .build()
                .show(Some(&self.window));
            return;
        }
        let dialog = gtk::Window::builder()
            .transient_for(&self.window)
            .modal(true)
            .destroy_with_parent(true)
            .default_width(425)
            .title(self.text("Equalizer", "Equalizador"))
            .build();
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 6);
        panel.add_css_class("content");
        let settings = self.state.borrow().equalizer;
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let enabled =
            gtk::CheckButton::with_label(self.text("Enable equalizer", "Ativar equalizador"));
        enabled.set_active(settings.enabled);
        enabled.set_hexpand(true);
        toolbar.append(&enabled);
        let reset = gtk::Button::with_label(self.text("Reset gains", "Zerar ganhos"));
        toolbar.append(&reset);
        panel.append(&toolbar);
        let mut names = vec![self.text("Custom", "Personalizado")];
        names.extend(
            equalizer::PRESETS
                .iter()
                .map(|(en, pt, _)| self.text(en, pt)),
        );
        let preset = gtk::DropDown::from_strings(&names);
        preset.update_property(&[gtk::accessible::Property::Label(
            self.text("Equalizer preset", "Predefinição do equalizador"),
        )]);
        let selection = equalizer::PRESETS
            .iter()
            .position(|(_, _, gains)| *gains == settings.gains);
        preset.set_selected(selection.map_or(0, |index| index as u32 + 1));
        panel.append(&preset);
        let applying = Rc::new(Cell::new(false));
        let mut controls = Vec::new();
        for (index, frequency) in equalizer::BANDS.iter().enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.append(&table_label(frequency, 68));
            let gain = gtk::Scale::with_range(gtk::Orientation::Horizontal, -24.0, 12.0, 1.0);
            gain.set_hexpand(true);
            gain.set_draw_value(true);
            gain.set_digits(0);
            gain.set_value_pos(gtk::PositionType::Right);
            gain.set_format_value_func(|_, value| format!("{value:+.0} dB"));
            gain.add_mark(0.0, gtk::PositionType::Bottom, None);
            gain.set_value(settings.gains[index]);
            gain.update_property(&[gtk::accessible::Property::Label(frequency)]);
            let weak = Rc::downgrade(self);
            let preset = preset.downgrade();
            let applying_preset = applying.clone();
            gain.connect_value_changed(move |slider| {
                if applying_preset.get() {
                    return;
                }
                if let Some(preset) = preset.upgrade() {
                    preset.set_selected(0);
                }
                if let Some(player) = weak.upgrade() {
                    let mut state = player.state.borrow_mut();
                    state.equalizer.gains[index] = slider.value();
                    state
                        .equalizer
                        .apply(player.equalizer.as_ref().expect("checked when opening"));
                }
            });
            row.append(&gain);
            panel.append(&row);
            controls.push(gain);
        }
        let weak = Rc::downgrade(self);
        let preset_enable = enabled.clone();
        preset.connect_selected_notify(move |preset| {
            let Some(index) = preset.selected().checked_sub(1) else {
                return;
            };
            let Some((_, _, gains)) = equalizer::PRESETS.get(index as usize) else {
                return;
            };
            let Some(player) = weak.upgrade() else { return };
            applying.set(true);
            for (control, value) in controls.iter().zip(gains) {
                control.set_value(*value);
            }
            applying.set(false);
            if index > 0 {
                preset_enable.set_active(true);
            }
            let mut state = player.state.borrow_mut();
            state.equalizer.gains = *gains;
            state
                .equalizer
                .apply(player.equalizer.as_ref().expect("checked when opening"));
        });
        let selection = preset.downgrade();
        reset.connect_clicked(move |_| {
            if let Some(preset) = selection.upgrade() {
                preset.set_selected(1);
            }
        });
        let weak = Rc::downgrade(self);
        enabled.connect_toggled(move |enabled| {
            if let Some(player) = weak.upgrade() {
                let mut state = player.state.borrow_mut();
                state.equalizer.enabled = enabled.is_active();
                state
                    .equalizer
                    .apply(player.equalizer.as_ref().expect("checked when opening"));
            }
        });
        let close = gtk::Button::with_label(self.text("Close", "Fechar"));
        let window = dialog.downgrade();
        close.connect_clicked(move |_| {
            if let Some(window) = window.upgrade() {
                window.close();
            }
        });
        panel.append(&close);
        let weak = Rc::downgrade(self);
        dialog.connect_close_request(move |_| {
            if let Some(player) = weak.upgrade() {
                player.save();
            }
            glib::Propagation::Proceed
        });
        dialog.set_child(Some(&panel));
        dialog.present();
    }

    pub(crate) fn show_view_settings(self: &Rc<Self>) {
        let window = gtk::Window::builder()
            .title(self.text("View", "Visualização"))
            .transient_for(&self.window)
            .modal(true)
            .destroy_with_parent(true)
            .default_height(425)
            .build();
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 8);
        panel.add_css_class("content");
        let controls = self.view_controls();
        controls.set_vexpand(true);
        panel.append(&controls);
        let close = gtk::Button::with_label(self.text("Close", "Fechar"));
        close.set_halign(gtk::Align::End);
        let weak = window.downgrade();
        close.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                window.close();
            }
        });
        panel.append(&close);
        window.set_child(Some(&panel));
        window.present();
    }

    pub(crate) fn view_controls(self: &Rc<Self>) -> gtk::Box {
        #[derive(Clone, Copy, PartialEq)]
        enum Scope {
            General,
            Search,
            Column(usize),
            Playlist,
            Lyrics,
            OnlineLyrics,
        }
        type ViewField = fn(&mut ViewOptions) -> &mut bool;
        let choices: [(&str, ViewField, Scope); 19] = [
            (
                self.text("Show sidebar", "Mostrar menu lateral"),
                |view| &mut view.sidebar,
                Scope::General,
            ),
            (
                self.text("Search as a button", "Busca como botão"),
                |view| &mut view.search_button,
                Scope::Search,
            ),
            (
                self.text(
                    "Page title in the top bar",
                    "Título da página na barra superior",
                ),
                |view| &mut view.titlebar_heading,
                Scope::General,
            ),
            (
                self.text("Show title bar icon", "Mostrar ícone na barra de título"),
                |view| &mut view.titlebar_icon,
                Scope::General,
            ),
            (
                self.text("Show artist", "Mostrar artista"),
                |view| &mut view.columns[0],
                Scope::Column(1),
            ),
            (
                self.text("Show album", "Mostrar álbum"),
                |view| &mut view.columns[1],
                Scope::Column(2),
            ),
            (
                self.text("Show track length", "Mostrar duração"),
                |view| &mut view.columns[2],
                Scope::Column(3),
            ),
            (
                self.text("Show library buttons", "Mostrar botões da biblioteca"),
                |view| &mut view.library_actions,
                Scope::General,
            ),
            (
                self.text(
                    "Show playlist buttons",
                    "Mostrar botões da lista de reprodução",
                ),
                |view| &mut view.playlist_actions,
                Scope::Playlist,
            ),
            (
                self.text(
                    "Show playlist drop area",
                    "Mostrar área de soltar da lista de reprodução",
                ),
                |view| &mut view.playlist_drop_zone,
                Scope::Playlist,
            ),
            (
                self.text("Show cover thumbnail", "Mostrar miniatura da capa"),
                |view| &mut view.cover_thumbnail,
                Scope::General,
            ),
            (
                self.text("Show playing track", "Mostrar faixa em reprodução"),
                |view| &mut view.track_info,
                Scope::General,
            ),
            (
                self.text("Show seek bar", "Mostrar barra de progresso"),
                |view| &mut view.seek_bar,
                Scope::General,
            ),
            (
                self.text("Show playback time", "Mostrar tempo de reprodução"),
                |view| &mut view.elapsed,
                Scope::General,
            ),
            (
                self.text("Show shuffle and repeat", "Mostrar aleatório e repetição"),
                |view| &mut view.playback_modes,
                Scope::General,
            ),
            (
                self.text("Show volume", "Mostrar volume"),
                |view| &mut view.volume,
                Scope::General,
            ),
            (
                self.text("Show volume percentage", "Mostrar porcentagem do volume"),
                |view| &mut view.volume_percent,
                Scope::General,
            ),
            (
                self.text("Enable lyrics (Ctrl+Y)", "Ativar letras (Ctrl+Y)"),
                |view| &mut view.lyrics,
                Scope::Lyrics,
            ),
            (
                self.text("Look up lyrics online", "Buscar letras online"),
                |view| &mut view.online_lyrics,
                Scope::OnlineLyrics,
            ),
        ];
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 4);
        panel.add_css_class("view-options");
        let mut current = self.state.borrow().view;
        let mut options = Vec::new();
        for (label, field, scope) in choices {
            let preferred = *field(&mut current);
            let auto_hidden = match scope {
                Scope::Search if self.track_search_field.borrow().is_none() => continue,
                Scope::Playlist
                    if !matches!(self.page.get(), Page::Playlists | Page::Playlist(_)) =>
                {
                    continue;
                }
                Scope::Column(index) => {
                    let columns = self.columns.borrow();
                    let Some(columns) = columns.as_ref() else {
                        continue;
                    };
                    preferred && !columns.column_visible(index)
                }
                _ => false,
            };
            let item = gtk::CheckButton::with_label(label);
            if scope == Scope::OnlineLyrics {
                item.set_tooltip_text(Some(self.text(
                    "Send track tags and duration to LRCLIB when local lyrics are missing",
                    "Envia tags e duração da faixa ao LRCLIB quando não há letra local",
                )));
            }
            item.set_active(preferred && !auto_hidden);
            if auto_hidden {
                item.set_sensitive(false);
                item.set_tooltip_text(Some(self.text(
                    "Widen the window to show this column",
                    "Amplie a janela para mostrar esta coluna",
                )));
            }
            let weak = Rc::downgrade(self);
            item.connect_toggled(move |item| {
                if let Some(player) = weak.upgrade() {
                    match scope {
                        Scope::Lyrics => return player.set_lyrics_enabled(item.is_active()),
                        Scope::OnlineLyrics => {
                            return player.set_online_lyrics_enabled(item.is_active());
                        }
                        _ => {}
                    }
                    *field(&mut player.state.borrow_mut().view) = item.is_active();
                    if scope == Scope::Search {
                        player.track_search_open.set(SearchVisibility::Automatic);
                    }
                    player.apply_view();
                    player.save();
                    player.show(player.page.get());
                }
            });
            panel.append(&item);
            options.push((search::search_key(label), item.downgrade()));
        }
        let empty = gtk::Label::new(Some(
            self.text("No options found", "Nenhuma opção encontrada"),
        ));
        empty.add_css_class("dim-label");
        empty.set_visible(false);
        panel.append(&empty);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some(self.text("Search options", "Buscar opções")));
        let empty = empty.downgrade();
        search.connect_search_changed(move |entry| {
            let terms = search::search_key(&entry.text());
            let mut found = false;
            for (label, option) in &options {
                if let Some(option) = option.upgrade() {
                    let matches = terms.split_whitespace().all(|term| label.contains(term));
                    option.set_visible(matches);
                    found |= matches;
                }
            }
            if let Some(empty) = empty.upgrade() {
                empty.set_visible(!found);
            }
        });
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_height(290)
            .max_content_height(400)
            .propagate_natural_height(true)
            .propagate_natural_width(true)
            .build();
        scroll.set_child(Some(&panel));
        let controls = gtk::Box::new(gtk::Orientation::Vertical, 8);
        controls.append(&search);
        controls.append(&scroll);
        controls
    }

    pub(crate) fn apply_view(&self) {
        let view = self.state.borrow().view;
        self.fit_sidebar();
        set_visible(&self.app_icon, view.titlebar_icon);
        self.artwork_button.set_visible(view.cover_thumbnail);
        self.track_caption.set_visible(view.track_info);
        self.title_grip.set_visible(view.track_info);
        self.fit_transport();
        self.fit_search();
        self.time.set_visible(view.elapsed);
        self.shuffle.set_visible(view.playback_modes);
        self.repeat.set_visible(view.playback_modes);
        self.volume.set_visible(view.volume);
        self.volume.set_draw_value(view.volume_percent);
        self.volume
            .set_width_request(if view.volume_percent { 90 } else { 66 });
        self.fit_columns();
        self.update_lyrics();
    }

    pub(crate) fn fit_sidebar(&self) {
        let visible = self.state.borrow().view.sidebar;
        if self.sidebar.is_visible() != visible {
            self.sidebar.set_visible(visible);
        }
    }

    pub(crate) fn preferences(self: &Rc<Self>) {
        self.heading(self.text("Preferences", "Preferências"));
        let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let hint = |text: &str| {
            let label = gtk::Label::new(Some(text));
            label.set_xalign(0.0);
            label.set_wrap(true);
            label.add_css_class("muted");
            form.append(&label);
        };
        hint(self.text("Display language", "Idioma da interface"));
        let selector = gtk::DropDown::from_strings(&[
            self.text("Use system language", "Usar idioma do sistema"),
            "English",
            "Português (Brasil)",
            "Español",
            "Français",
        ]);
        selector.add_css_class("preferences-control");
        selector.set_halign(gtk::Align::Start);
        selector.set_selected(match self.state.borrow().language {
            Language::System => 0,
            Language::English => 1,
            Language::Portuguese => 2,
            Language::Spanish => 3,
            Language::French => 4,
        });
        let weak = Rc::downgrade(self);
        selector.connect_selected_notify(move |selector| {
            if let Some(player) = weak.upgrade() {
                player.state.borrow_mut().language = match selector.selected() {
                    1 => Language::English,
                    2 => Language::Portuguese,
                    3 => Language::Spanish,
                    4 => Language::French,
                    _ => Language::System,
                };
                player.save();
                let _ = gtk::prelude::WidgetExt::activate_action(
                    &player.window,
                    "win.refresh-language",
                    None,
                );
            }
        });
        form.append(&selector);

        hint(self.text("Appearance", "Aparência"));
        let theme = gtk::DropDown::from_strings(&[
            self.text("Follow system", "Seguir sistema"),
            self.text("Light", "Claro"),
            self.text("Dark", "Escuro"),
            APP_NAME,
        ]);
        theme.add_css_class("preferences-control");
        theme.set_halign(gtk::Align::Start);
        theme.set_selected(match self.state.borrow().theme {
            Theme::System => 0,
            Theme::Light => 1,
            Theme::Dark => 2,
            Theme::Ferritone => 3,
        });
        let weak = Rc::downgrade(self);
        theme.connect_selected_notify(move |theme| {
            let Some(player) = weak.upgrade() else { return };
            player.state.borrow_mut().theme = match theme.selected() {
                1 => Theme::Light,
                2 => Theme::Dark,
                3 => Theme::Ferritone,
                _ => Theme::System,
            };
            player.apply_theme();
            player.save();
        });
        form.append(&theme);

        hint(self.text("Accent color", "Cor de destaque"));
        let accent = gtk::DropDown::from_strings(&[
            self.text("Follow system", "Seguir sistema"),
            self.text("Green", "Verde"),
            self.text("Blue", "Azul"),
            self.text("Violet", "Violeta"),
            self.text("Copper", "Cobre"),
        ]);
        accent.add_css_class("preferences-control");
        accent.set_halign(gtk::Align::Start);
        accent.set_selected(match self.state.borrow().accent {
            Accent::System => 0,
            Accent::Green => 1,
            Accent::Blue => 2,
            Accent::Violet => 3,
            Accent::Copper => 4,
        });
        let weak = Rc::downgrade(self);
        accent.connect_selected_notify(move |accent| {
            let Some(player) = weak.upgrade() else { return };
            player.state.borrow_mut().accent = match accent.selected() {
                1 => Accent::Green,
                2 => Accent::Blue,
                3 => Accent::Violet,
                4 => Accent::Copper,
                _ => Accent::System,
            };
            player.apply_theme();
            player.save();
        });
        form.append(&accent);

        let menu_icons = gtk::CheckButton::with_label(self.text(
            "Show icons in context menus",
            "Mostrar ícones nos menus de contexto",
        ));
        menu_icons.set_active(self.state.borrow().view.context_menu_icons);
        let weak = Rc::downgrade(self);
        menu_icons.connect_toggled(move |button| {
            if let Some(player) = weak.upgrade() {
                player.state.borrow_mut().view.context_menu_icons = button.is_active();
                player.save();
            }
        });
        form.append(&menu_icons);
        let menu_keycaps = gtk::CheckButton::with_label(self.text(
            "Show keycaps for menu shortcuts",
            "Mostrar teclas nos atalhos dos menus",
        ));
        menu_keycaps.set_active(self.state.borrow().view.context_menu_keycaps);
        let weak = Rc::downgrade(self);
        menu_keycaps.connect_toggled(move |button| {
            if let Some(player) = weak.upgrade() {
                player.state.borrow_mut().view.context_menu_keycaps = button.is_active();
                player.save();
            }
        });
        form.append(&menu_keycaps);

        let heading = gtk::Label::new(Some(self.text("Lyrics", "Letras")));
        heading.add_css_class("section-title");
        heading.set_xalign(0.0);
        form.append(&heading);
        let view = self.state.borrow().view;
        let enabled = gtk::CheckButton::with_label(self.text("Enable lyrics", "Ativar letras"));
        let online = gtk::CheckButton::with_label(
            self.text("Look up lyrics online", "Buscar letras online"),
        );
        enabled.set_active(view.lyrics);
        online.set_active(view.online_lyrics);
        enabled
            .bind_property("active", &online, "sensitive")
            .sync_create()
            .build();
        self.lyrics_preference.set(Some(&enabled));
        self.online_lyrics_preference.set(Some(&online));
        let weak = Rc::downgrade(self);
        enabled.connect_toggled(move |button| {
            if let Some(player) = weak.upgrade() {
                player.set_lyrics_enabled(button.is_active());
            }
        });
        let weak = Rc::downgrade(self);
        online.connect_toggled(move |button| {
            if let Some(player) = weak.upgrade() {
                player.set_online_lyrics_enabled(button.is_active());
            }
        });
        form.append(&enabled);
        form.append(&online);
        hint(self.text(
            "Online lookup sends track tags and duration to LRCLIB.",
            "A busca online envia as tags e a duração da faixa ao LRCLIB.",
        ));
        self.audio_preferences(&form);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&form)
            .build();
        self.content.append(&scroll);
    }

    pub(crate) fn audio_preferences(self: &Rc<Self>, form: &gtk::Box) {
        use replay_gain::Mode;
        let audio = gtk::Label::new(Some(self.text("Audio", "Áudio")));
        audio.add_css_class("section-title");
        audio.set_xalign(0.0);
        form.append(&audio);
        let mode_label = gtk::Label::new(Some("ReplayGain"));
        mode_label.set_xalign(0.0);
        form.append(&mode_label);
        let gain = gtk::DropDown::from_strings(&[
            self.text("Off", "Desativado"),
            self.text("Track gain", "Ganho por faixa"),
            self.text("Album gain", "Ganho por álbum"),
        ]);
        gain.add_css_class("preferences-control");
        gain.set_halign(gtk::Align::Start);
        let saved = self.state.borrow().replay_gain;
        let index = match saved.mode {
            Mode::Off => 0,
            Mode::Track => 1,
            Mode::Album => 2,
        };
        gain.set_selected(index);
        let has_gain = self.replay_gain.is_some();
        gain.set_sensitive(has_gain);
        let protect = gtk::CheckButton::with_label(self.text(
            "Limit gain to avoid clipping",
            "Limitar o ganho para evitar distorção",
        ));
        protect.set_active(saved.protect_peaks);
        protect.set_sensitive(has_gain && index != 0);
        let weak = Rc::downgrade(self);
        let peak_toggle = protect.clone();
        gain.connect_selected_notify(move |gain| {
            let Some(player) = weak.upgrade() else { return };
            player.state.borrow_mut().replay_gain.mode = match gain.selected() {
                1 => Mode::Track,
                2 => Mode::Album,
                _ => Mode::Off,
            };
            peak_toggle.set_sensitive(has_gain && gain.selected() != 0);
            player.save_replay_gain();
        });
        let weak = Rc::downgrade(self);
        protect.connect_toggled(move |protect| {
            if let Some(player) = weak.upgrade() {
                player.state.borrow_mut().replay_gain.protect_peaks = protect.is_active();
                player.save_replay_gain();
            }
        });
        form.append(&gain);
        form.append(&protect);
        let description = gtk::Label::new(Some(if has_gain {
            self.text(
                "ReplayGain levels volume using the audio file's gain tags.",
                "O ReplayGain nivela o volume usando as tags de ganho do arquivo.",
            )
        } else {
            self.text("ReplayGain is unavailable.", "ReplayGain indisponível.")
        }));
        description.set_xalign(0.0);
        description.set_wrap(true);
        form.append(&description);
    }

    pub(crate) fn save_replay_gain(&self) {
        if let Some(replay_gain) = &self.replay_gain {
            replay_gain.apply(self.state.borrow().replay_gain);
        }
        self.save();
    }

    pub(crate) fn apply_theme(&self) {
        let state = self.state.borrow();
        if let Some(settings) = gtk::Settings::default() {
            settings.set_gtk_application_prefer_dark_theme(match state.theme {
                Theme::System => match self
                    .desktop_appearance
                    .as_ref()
                    .map(|appearance| appearance.string("color-scheme"))
                    .as_deref()
                {
                    Some("prefer-dark") => true,
                    Some("prefer-light") => false,
                    _ => self.default_dark,
                },
                Theme::Light => false,
                Theme::Ferritone | Theme::Dark => true,
            });
        }
        let desktop_accent = self
            .desktop_appearance
            .as_ref()
            .filter(|settings| {
                settings
                    .settings_schema()
                    .is_some_and(|schema| schema.has_key("accent-color"))
            })
            .map(|settings| settings.string("accent-color"));
        let accent = state.accent.color(desktop_accent.as_deref());
        let foreground = match accent {
            "@theme_selected_bg_color" => "@theme_selected_fg_color",
            "#9141ac" => "#ffffff",
            _ => "#090b09",
        };
        let (background, text) = if matches!(state.theme, Theme::Ferritone) {
            self.window.add_css_class("ferritone");
            ("#1e1d1b", "#f5f3ef")
        } else {
            self.window.remove_css_class("ferritone");
            ("@theme_bg_color", "@theme_fg_color")
        };
        self.css.load_from_data(
            &include_str!("style.css")
                .replace("PLAYER_BACKGROUND", background)
                .replace("PLAYER_TEXT", text)
                .replace("ACCENT_COLOR", accent)
                .replace("ACCENT_FOREGROUND", foreground),
        );
    }
}
