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

    /// Opens Preferences, or does nothing when it is already open. The dialog is built once
    /// and kept, so opening it only runs its animation.
    pub(crate) fn show_preferences(self: &Rc<Self>) {
        let dialog = self.preferences_dialog();
        if dialog.parent().is_none() {
            dialog.present(Some(&self.window));
        }
    }

    /// The Preferences dialog, built the first time it is needed.
    pub(crate) fn preferences_dialog(self: &Rc<Self>) -> adw::PreferencesDialog {
        if let Some(dialog) = self.preferences_dialog.borrow().as_ref() {
            return dialog.clone();
        }
        let dialog = adw::PreferencesDialog::new();
        dialog.set_title(self.text("Preferences", "Preferências"));
        dialog.add(&self.preferences_page());
        self.preferences_dialog.replace(Some(dialog.clone()));
        dialog
    }

    /// Builds Preferences while the window is idle, so the first opening does not wait for it.
    pub(crate) fn prepare_preferences(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::idle_add_local_full(glib::Priority::LOW, move || {
            if let Some(player) = weak.upgrade() {
                player.preferences_dialog();
            }
            glib::ControlFlow::Break
        });
    }

    /// Rebuilds Preferences in the new language.
    pub(crate) fn reopen_preferences(self: &Rc<Self>) {
        let Some(dialog) = self.preferences_dialog.borrow().clone() else {
            return;
        };
        let weak = Rc::downgrade(self);
        // The language row that asked for this is still emitting its signal.
        glib::idle_add_local_once(move || {
            let Some(player) = weak.upgrade() else { return };
            if let Some(page) = dialog.visible_page() {
                dialog.remove(&page);
            }
            dialog.set_title(player.text("Preferences", "Preferências"));
            dialog.add(&player.preferences_page());
        });
    }

    fn preferences_page(self: &Rc<Self>) -> adw::PreferencesPage {
        let page = adw::PreferencesPage::new();
        page.set_title(self.text("Preferences", "Preferências"));
        page.set_icon_name(Some("preferences-system-symbolic"));

        let general = adw::PreferencesGroup::new();
        let selector = combo_row(
            self.text("Display language", "Idioma da interface"),
            &[
                self.text("Use system language", "Usar idioma do sistema"),
                "English",
                "Português (Brasil)",
                "Español",
                "Français",
            ],
        );
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
        general.add(&selector);
        page.add(&general);

        let appearance = adw::PreferencesGroup::new();
        appearance.set_title(self.text("Appearance", "Aparência"));
        let theme = combo_row(self.text("Theme", "Tema"), &[APP_NAME, "GNOME/Adwaita"]);
        theme.set_selected(match self.state.borrow().theme {
            Theme::Ferritone => 0,
            _ => 1,
        });
        let scheme = combo_row(
            self.text("Color scheme", "Esquema de cores"),
            &[
                self.text("Follow system", "Seguir sistema"),
                self.text("Light", "Claro"),
                self.text("Dark", "Escuro"),
            ],
        );
        scheme.set_selected(match self.state.borrow().color_scheme {
            ColorScheme::System => 0,
            ColorScheme::Light => 1,
            ColorScheme::Dark => 2,
        });
        let accent = combo_row(
            self.text("Accent color", "Cor de destaque"),
            &[
                self.text("Follow system", "Seguir sistema"),
                self.text("Green", "Verde"),
                self.text("Blue", "Azul"),
                self.text("Violet", "Violeta"),
                self.text("Copper", "Cobre"),
            ],
        );
        accent.set_selected(match self.state.borrow().accent {
            Accent::System => 0,
            Accent::Green => 1,
            Accent::Blue => 2,
            Accent::Violet => 3,
            Accent::Copper => 4,
        });
        // Only the rows the theme uses are shown: Ferritone is always dark, and GNOME/Adwaita
        // always takes the desktop's accent. The theme row says what the theme does.
        let fit_rows = {
            let (theme, scheme, accent) =
                (theme.downgrade(), scheme.downgrade(), accent.downgrade());
            let weak = Rc::downgrade(self);
            move || {
                let (Some(player), Some(row), Some(scheme), Some(accent)) = (
                    weak.upgrade(),
                    theme.upgrade(),
                    scheme.upgrade(),
                    accent.upgrade(),
                ) else {
                    return;
                };
                let theme = player.state.borrow().theme;
                let ferritone = matches!(theme, Theme::Ferritone);
                row.set_subtitle(if ferritone {
                    player.text(
                        "Charcoal and copper, always dark.",
                        "Carvão e cobre, sempre escuro.",
                    )
                } else {
                    player.text(
                        "GNOME's own look, with the system accent color.",
                        "O visual do próprio GNOME, com a cor de destaque do sistema.",
                    )
                });
                scheme.set_visible(!ferritone);
                accent.set_visible(ferritone);
            }
        };
        fit_rows();
        let weak = Rc::downgrade(self);
        theme.connect_selected_notify(move |theme| {
            let Some(player) = weak.upgrade() else { return };
            player.state.borrow_mut().theme = match theme.selected() {
                1 => Theme::Adwaita,
                _ => Theme::Ferritone,
            };
            fit_rows();
            player.apply_theme();
            player.save();
        });
        let weak = Rc::downgrade(self);
        scheme.connect_selected_notify(move |scheme| {
            let Some(player) = weak.upgrade() else { return };
            player.state.borrow_mut().color_scheme = match scheme.selected() {
                1 => ColorScheme::Light,
                2 => ColorScheme::Dark,
                _ => ColorScheme::System,
            };
            player.apply_theme();
            player.save();
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
        appearance.add(&theme);
        appearance.add(&scheme);
        appearance.add(&accent);
        let view = self.state.borrow().view;
        appearance.add(&self.switch_row(
            self.text(
                "Show icons in context menus",
                "Mostrar ícones nos menus de contexto",
            ),
            view.context_menu_icons,
            |player, active| player.state.borrow_mut().view.context_menu_icons = active,
        ));
        appearance.add(&self.switch_row(
            self.text(
                "Show keycaps for menu shortcuts",
                "Mostrar teclas nos atalhos dos menus",
            ),
            view.context_menu_keycaps,
            |player, active| player.state.borrow_mut().view.context_menu_keycaps = active,
        ));
        page.add(&appearance);

        let lyrics = adw::PreferencesGroup::new();
        lyrics.set_title(self.text("Lyrics", "Letras"));
        let enabled = adw::SwitchRow::builder()
            .title(self.text("Enable lyrics", "Ativar letras"))
            .active(view.lyrics)
            .build();
        let online = adw::SwitchRow::builder()
            .title(self.text("Look up lyrics online", "Buscar letras online"))
            .subtitle(self.text(
                "Online lookup sends track tags and duration to LRCLIB.",
                "A busca online envia as tags e a duração da faixa ao LRCLIB.",
            ))
            .active(view.online_lyrics)
            .build();
        enabled
            .bind_property("active", &online, "sensitive")
            .sync_create()
            .build();
        self.lyrics_preference.set(Some(&enabled));
        self.online_lyrics_preference.set(Some(&online));
        let weak = Rc::downgrade(self);
        enabled.connect_active_notify(move |row| {
            if let Some(player) = weak.upgrade() {
                player.set_lyrics_enabled(row.is_active());
            }
        });
        let weak = Rc::downgrade(self);
        online.connect_active_notify(move |row| {
            if let Some(player) = weak.upgrade() {
                player.set_online_lyrics_enabled(row.is_active());
            }
        });
        lyrics.add(&enabled);
        lyrics.add(&online);
        page.add(&lyrics);
        page.add(&self.audio_preferences());
        page.add(&self.listenbrainz_preferences());
        page.add(&self.browser_shortcut_preferences());
        page
    }

    /// A switch that saves its setting as it changes.
    fn switch_row(
        self: &Rc<Self>,
        title: &str,
        active: bool,
        store: impl Fn(&Player, bool) + 'static,
    ) -> adw::SwitchRow {
        let row = adw::SwitchRow::builder()
            .title(title)
            .active(active)
            .build();
        let weak = Rc::downgrade(self);
        row.connect_active_notify(move |row| {
            if let Some(player) = weak.upgrade() {
                store(&player, row.is_active());
                player.save();
            }
        });
        row
    }

    pub(crate) fn audio_preferences(self: &Rc<Self>) -> adw::PreferencesGroup {
        use replay_gain::Mode;
        let group = adw::PreferencesGroup::new();
        group.set_title(self.text("Audio", "Áudio"));
        let has_gain = self.replay_gain.is_some();
        let gain = combo_row(
            "ReplayGain",
            &[
                self.text("Off", "Desativado"),
                self.text("Track gain", "Ganho por faixa"),
                self.text("Album gain", "Ganho por álbum"),
            ],
        );
        gain.set_subtitle(if has_gain {
            self.text(
                "ReplayGain levels volume using the audio file's gain tags.",
                "O ReplayGain nivela o volume usando as tags de ganho do arquivo.",
            )
        } else {
            self.text("ReplayGain is unavailable.", "ReplayGain indisponível.")
        });
        let saved = self.state.borrow().replay_gain;
        let index = match saved.mode {
            Mode::Off => 0,
            Mode::Track => 1,
            Mode::Album => 2,
        };
        gain.set_selected(index);
        gain.set_sensitive(has_gain);
        let protect = adw::SwitchRow::builder()
            .title(self.text(
                "Limit gain to avoid clipping",
                "Limitar o ganho para evitar distorção",
            ))
            .active(saved.protect_peaks)
            .sensitive(has_gain && index != 0)
            .build();
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
        protect.connect_active_notify(move |protect| {
            if let Some(player) = weak.upgrade() {
                player.state.borrow_mut().replay_gain.protect_peaks = protect.is_active();
                player.save_replay_gain();
            }
        });
        group.add(&gain);
        group.add(&protect);
        let screen = adw::SwitchRow::builder()
            .title(self.text(
                "Keep the screen from locking while playing",
                "Não deixar a tela bloquear enquanto toca",
            ))
            .active(!self.state.borrow().allow_screen_lock)
            .build();
        let weak = Rc::downgrade(self);
        screen.connect_active_notify(move |screen| {
            if let Some(player) = weak.upgrade() {
                player.state.borrow_mut().allow_screen_lock = !screen.is_active();
                player.save();
                player.refresh_inhibit();
            }
        });
        group.add(&screen);
        let awake = adw::SwitchRow::builder()
            .title(self.text(
                "Keep the computer from sleeping while playing",
                "Não deixar o computador suspender enquanto toca",
            ))
            .subtitle(self.text(
                "Suspending would stop the music",
                "Suspender pararia a música",
            ))
            .active(!self.state.borrow().allow_sleep)
            .build();
        let weak = Rc::downgrade(self);
        awake.connect_active_notify(move |awake| {
            if let Some(player) = weak.upgrade() {
                player.state.borrow_mut().allow_sleep = !awake.is_active();
                player.save();
                player.refresh_inhibit();
            }
        });
        group.add(&awake);
        let gpu = self.switch_row(
            self.text(
                "Draw the interface with the graphics card",
                "Desenhar a interface com a placa de vídeo",
            ),
            self.state.borrow().gpu_rendering.0,
            |player, active| player.state.borrow_mut().gpu_rendering.0 = active,
        );
        gpu.set_subtitle(self.text(
            "Uses about 50 to 150 MB more memory. Takes effect after reopening Ferritone.",
            "Usa cerca de 50 a 150 MB a mais de memória. Vale depois de reabrir o Ferritone.",
        ));
        group.add(&gpu);
        group
    }

    pub(crate) fn save_replay_gain(&self) {
        if let Some(replay_gain) = &self.replay_gain {
            replay_gain.apply(self.state.borrow().replay_gain);
        }
        self.save();
    }

    /// Whether the chosen theme is dark, resolving "Follow system" from the desktop setting.
    pub(crate) fn theme_is_dark(&self) -> bool {
        let state = self.state.borrow();
        match (state.theme, state.color_scheme) {
            (Theme::Ferritone, _) => true,
            (_, ColorScheme::Light) => false,
            (_, ColorScheme::Dark) => true,
            (_, ColorScheme::System) => match self.desktop_color_scheme().as_deref() {
                Some("prefer-dark") => true,
                Some("prefer-light") => false,
                _ => adw::StyleManager::default().is_dark(),
            },
        }
    }

    /// The accent color and the text color that reads on it.
    pub(crate) fn accent_colors(&self) -> (&'static str, &'static str) {
        let accent = self
            .state
            .borrow()
            .accent
            .color(self.desktop_accent().as_deref());
        let foreground = match accent {
            "@accent_bg_color" => "@accent_fg_color",
            "#9141ac" => "#ffffff",
            _ => "#090b09",
        };
        (accent, foreground)
    }

    /// Concrete colors for pages outside GTK, such as the phone remote, matching the theme.
    pub(crate) fn palette(&self) -> remote::Palette {
        let system_accent = adw::StyleManager::default().accent_color_rgba();
        let system_accent = format!(
            "#{:02x}{:02x}{:02x}",
            (system_accent.red() * 255.0).round() as u8,
            (system_accent.green() * 255.0).round() as u8,
            (system_accent.blue() * 255.0).round() as u8,
        );
        let (accent, accent_text) = match self.accent_colors() {
            // GNOME/Adwaita and an unknown system accent take libadwaita's resolved accent.
            _ if !matches!(self.state.borrow().theme, Theme::Ferritone) => {
                (system_accent.as_str(), "#ffffff")
            }
            ("@accent_bg_color", _) => (system_accent.as_str(), "#ffffff"),
            colors => colors,
        };
        let dark = self.theme_is_dark();
        let [background, surface, raised, text, muted, line] =
            if matches!(self.state.borrow().theme, Theme::Ferritone) {
                [
                    "#1e1d1b", "#2a2926", "#34332f", "#f5f3ef", "#a8a39a", "#3d3b37",
                ]
            } else if dark {
                [
                    "#242424", "#2e2e2e", "#383838", "#ffffff", "#a0a0a0", "#454545",
                ]
            } else {
                [
                    "#fafafa", "#ebebeb", "#dedede", "#2e2e2e", "#6f6f6f", "#d4d4d4",
                ]
            };
        remote::Palette {
            dark,
            background: background.into(),
            surface: surface.into(),
            raised: raised.into(),
            text: text.into(),
            muted: muted.into(),
            line: line.into(),
            accent: accent.into(),
            accent_text: accent_text.into(),
        }
    }

    pub(crate) fn apply_theme(&self) {
        let (theme, scheme) = {
            let state = self.state.borrow();
            (state.theme, state.color_scheme)
        };
        adw::StyleManager::default().set_color_scheme(match (theme, scheme) {
            (Theme::Ferritone, _) | (_, ColorScheme::Dark) => adw::ColorScheme::ForceDark,
            (_, ColorScheme::Light) => adw::ColorScheme::ForceLight,
            (_, ColorScheme::System) => adw::ColorScheme::Default,
        });
        let ferritone = matches!(theme, Theme::Ferritone);
        // Every window, including the mini player, follows the theme as it changes.
        for window in gtk::Window::list_toplevels() {
            if let Ok(window) = window.downcast::<gtk::Window>() {
                if ferritone {
                    window.add_css_class("ferritone");
                } else {
                    window.remove_css_class("ferritone");
                }
            }
        }
        if !ferritone {
            // Only the layout stays; libadwaita draws every color.
            self.css.load_from_data("");
            return;
        }
        let (accent, foreground) = self.accent_colors();
        // libadwaita's own widgets, such as dialogs, rows and popovers, take the charcoal too.
        let mut colors = String::from(FERRITONE_COLORS);
        // The chosen accent also colors libadwaita's switches, links and suggested buttons.
        // The desktop's own accent is already libadwaita's, and naming it again would loop.
        if !accent.starts_with('@') {
            colors.push_str(&format!(
                "@define-color accent_bg_color {accent};\n\
             @define-color accent_fg_color {foreground};\n\
             @define-color accent_color {accent};\n"
            ));
        }
        colors.push_str(
            &include_str!("style.css")
                .replace("PLAYER_BACKGROUND", "#1e1d1b")
                .replace("PLAYER_TEXT", "#f5f3ef")
                .replace("ACCENT_COLOR", accent)
                .replace("ACCENT_FOREGROUND", foreground),
        );
        self.css.load_from_data(&colors);
    }

    /// Widens the sidebar to its longest page name, such as "Listas de reprodução", within
    /// limits, so names are not cut off; the split view would otherwise keep its minimum width.
    pub(crate) fn fit_sidebar_width(&self) {
        let natural = self.sidebar.measure(gtk::Orientation::Horizontal, -1).1;
        let width = f64::from(natural).clamp(172.0, 260.0);
        if self.split_view.min_sidebar_width() != width {
            self.split_view.set_min_sidebar_width(width);
        }
    }

    pub(crate) fn fit_sidebar(&self) {
        // A narrow window folds the sidebar away; it opens over the page from the header.
        let visible = self.state.borrow().view.sidebar && !self.split_view.is_collapsed();
        if self.split_view.shows_sidebar() != visible {
            self.split_view.set_show_sidebar(visible);
        }
    }
}

/// libadwaita's named colors in Ferritone's charcoal.
const FERRITONE_COLORS: &str = "\
@define-color window_bg_color #1e1d1b;
@define-color window_fg_color #f5f3ef;
@define-color view_bg_color #1e1d1b;
@define-color view_fg_color #f5f3ef;
@define-color headerbar_bg_color #36342f;
@define-color headerbar_fg_color #f5f3ef;
@define-color headerbar_backdrop_color #2a2926;
@define-color sidebar_bg_color #23221f;
@define-color sidebar_fg_color #f5f3ef;
@define-color sidebar_backdrop_color #23221f;
@define-color popover_bg_color #2a2926;
@define-color popover_fg_color #f5f3ef;
@define-color dialog_bg_color #2a2926;
@define-color dialog_fg_color #f5f3ef;
@define-color card_bg_color rgba(255, 255, 255, 0.05);
@define-color card_fg_color #f5f3ef;
";

/// A row that picks one of `options`.
fn combo_row(title: &str, options: &[&str]) -> adw::ComboRow {
    adw::ComboRow::builder()
        .title(title)
        .model(&gtk::StringList::new(options))
        .build()
}
