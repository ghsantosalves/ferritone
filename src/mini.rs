use std::{cell::Cell, rc::Rc};

use gtk::{glib, prelude::*};

use crate::{
    APP_ID, APP_NAME, Player,
    library::{MiniDisplay, MiniVisual, Theme},
    waveform::{POINTS, WaveformFrame},
};

pub fn open(player: &Rc<Player>) {
    if let Some(window) = player.mini.borrow().as_ref() {
        window.present();
        return;
    }
    let Some(app) = player.window.application() else {
        return;
    };
    let window = gtk::ApplicationWindow::builder()
        .application(&app)
        .icon_name(APP_ID)
        .title(format!(
            "{APP_NAME} · {}",
            player.text("Mini player", "Miniplayer")
        ))
        .default_width(385)
        .default_height(365)
        .build();
    if matches!(player.state.borrow().theme, Theme::Ferritone) {
        window.add_css_class("ferritone");
    }
    let header = adw::HeaderBar::new();
    header.add_css_class("compact-titlebar");
    let heading = gtk::Label::new(Some(player.text("Mini player", "Miniplayer")));
    heading.add_css_class("title");
    header.set_title_widget(Some(&heading));
    let icon = gtk::Image::from_icon_name(APP_ID);
    icon.set_pixel_size(24);
    player
        .app_icon
        .bind_property("visible", &icon, "visible")
        .sync_create()
        .build();
    header.pack_start(&icon);
    let restore = button(
        "view-fullscreen-symbolic",
        player.text("Return to main window", "Voltar à janela principal"),
    );
    let mini = window.downgrade();
    restore.connect_clicked(move |_| {
        if let Some(mini) = mini.upgrade() {
            mini.close();
        }
    });
    header.pack_end(&restore);
    window.set_titlebar(Some(&header));

    let layout = gtk::Box::new(gtk::Orientation::Vertical, 7);
    layout.add_css_class("mini-layout");
    let cover = gtk::Overlay::new();
    cover.set_overflow(gtk::Overflow::Hidden);
    cover.add_css_class("mini-cover");
    let placeholder = gtk::Image::from_icon_name("audio-x-generic-symbolic");
    placeholder.set_pixel_size(64);
    placeholder.add_css_class("mini-visual");
    cover.set_child(Some(&placeholder));
    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_can_shrink(true);
    picture.set_alternative_text(Some(player.text("Album cover", "Capa do álbum")));
    player
        .cover_source
        .bind_property("paintable", &picture, "paintable")
        .sync_create()
        .build();
    picture
        .bind_property("paintable", &placeholder, "visible")
        .transform_to(|_, artwork: Option<gtk::gdk::Paintable>| Some(artwork.is_none()))
        .sync_create()
        .build();
    cover.add_overlay(&picture);
    cover.set_measure_overlay(&picture, false);
    cover.set_clip_overlay(&picture, true);

    // The waveform is a wide, short strip under the cover rather than a square beside it.
    let area = gtk::DrawingArea::new();
    area.set_content_width(90);
    area.set_content_height(56);
    area.add_css_class("mini-visual");
    area.add_css_class("mini-waveform");
    area.update_property(&[gtk::accessible::Property::Label(
        player.text("Audio waveform", "Ondas do áudio"),
    )]);
    let (saved, lyrics_enabled) = {
        let state = player.state.borrow();
        (
            state
                .mini_display
                .unwrap_or_else(|| state.mini_visual.into()),
            state.view.lyrics,
        )
    };
    let waveform_enabled = player.waveform.is_some();
    let fallback_cover = !saved.cover
        && !(saved.waveform && waveform_enabled)
        && !(saved.lyrics && lyrics_enabled)
        && (saved.waveform || saved.lyrics);
    let mode = Rc::new(Cell::new(saved.waveform_style));
    let frame = Rc::new(Cell::new(WaveformFrame::default()));
    let draw_mode = mode.clone();
    let draw_frame = frame.clone();
    let draw_player = Rc::downgrade(player);
    area.set_draw_func(move |_, context, width, height| {
        // The waveform takes the accent color chosen in Preferences.
        let accent = draw_player
            .upgrade()
            .and_then(|player| parse_hex(&player.palette().accent))
            .unwrap_or((0.96, 0.56, 0.30));
        draw_waveform(
            context,
            f64::from(width),
            f64::from(height),
            &draw_frame.get(),
            draw_mode.get(),
            accent,
        );
    });
    let visuals = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let artwork = gtk::Box::new(gtk::Orientation::Vertical, 10);
    artwork.set_vexpand(true);
    // A square frame keeps the cover's area the shape of the cover, so its rounded corners
    // land on the image rather than on the empty space beside it.
    cover.set_size_request(90, 90);
    let cover_frame = gtk::AspectFrame::new(0.5, 0.5, 1.0, false);
    cover_frame.set_child(Some(&cover));
    cover_frame.set_vexpand(true);
    area.set_hexpand(true);
    artwork.append(&cover_frame);
    artwork.append(&area);

    // With lyrics on screen, the cover shrinks to a thumbnail beside the track so the lyrics
    // get the room.
    let compact = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    compact.add_css_class("mini-header");
    let thumb = gtk::Picture::new();
    thumb.set_content_fit(gtk::ContentFit::Cover);
    thumb.set_can_shrink(true);
    thumb.set_overflow(gtk::Overflow::Hidden);
    thumb.add_css_class("mini-thumb");
    thumb.set_size_request(56, 56);
    thumb.set_valign(gtk::Align::Center);
    player
        .cover_source
        .bind_property("paintable", &thumb, "paintable")
        .sync_create()
        .build();
    compact.append(&thumb);
    let compact_info = gtk::Box::new(gtk::Orientation::Vertical, 2);
    compact_info.set_valign(gtk::Align::Center);
    compact_info.set_hexpand(true);
    for (source, style) in [
        (&player.now_playing, "playing-title"),
        (&player.playing_artist, "playing-artist"),
    ] {
        let label = gtk::Label::new(None);
        label.set_xalign(0.0);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.add_css_class(style);
        source
            .bind_property("label", &label, "label")
            .sync_create()
            .build();
        compact_info.append(&label);
    }
    compact.append(&compact_info);
    visuals.append(&compact);
    visuals.append(&artwork);
    let lyric_panel = gtk::Stack::new();
    lyric_panel.set_vhomogeneous(true);
    lyric_panel.set_vexpand(true);
    // The card fills the space; the verses stay centered inside it.
    lyric_panel.add_css_class("mini-lyric-card");
    lyric_panel.set_overflow(gtk::Overflow::Hidden);
    let excerpt = gtk::Box::new(gtk::Orientation::Vertical, 6);
    excerpt.add_css_class("mini-lyrics");
    excerpt.set_valign(gtk::Align::Center);
    excerpt.set_vexpand(true);
    player.attach_lyric_sync(&excerpt);
    for (index, source) in player.lyric_lines.iter().enumerate() {
        let lines = if index == 2 { 2 } else { 1 };
        let style = match index {
            2 => "lyric-current",
            1 | 3 => "lyric-nearby",
            _ => "lyric-distant",
        };
        // Only the blank sizing label contributes to the row's minimum size.
        let spacer = gtk::Label::builder()
            .label(if lines == 2 { " \n " } else { " " })
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build();
        spacer.add_css_class(style);
        let row = gtk::Overlay::new();
        row.set_child(Some(&spacer));
        let label = gtk::Label::new(None);
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_lines(lines);
        label.set_justify(gtk::Justification::Center);
        label.add_css_class(style);
        row.add_overlay(&label);
        row.set_clip_overlay(&label, true);
        source
            .bind_property("label", &label, "label")
            .sync_create()
            .build();
        if index == 2 {
            source
                .bind_property("opacity", &label, "opacity")
                .sync_create()
                .build();
        }
        excerpt.append(&row);
    }
    lyric_panel.add_named(&excerpt, Some("timed"));
    lyric_panel.add_named(&plain_lyrics(player), Some("plain"));
    player
        .mini_lyrics
        .bind_property("visible", &lyric_panel, "visible-child-name")
        .transform_to(|_, plain: bool| Some(if plain { "plain" } else { "timed" }))
        .sync_create()
        .build();
    visuals.append(&lyric_panel);
    layout.append(&visuals);

    let info = gtk::Box::new(gtk::Orientation::Vertical, 1);
    let title = gtk::Label::new(None);
    title.set_xalign(0.5);
    title.set_hexpand(true);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.add_css_class("playing-title");
    player
        .now_playing
        .bind_property("label", &title, "label")
        .sync_create()
        .build();
    player
        .now_playing
        .bind_property("tooltip-text", &title, "tooltip-text")
        .sync_create()
        .build();
    info.append(&title);
    let artist = gtk::Label::new(None);
    artist.add_css_class("playing-artist");
    artist.set_xalign(0.5);
    artist.set_ellipsize(gtk::pango::EllipsizeMode::End);
    for property in ["label", "tooltip-text", "visible"] {
        player
            .playing_artist
            .bind_property(property, &artist, property)
            .sync_create()
            .build();
    }
    info.append(&artist);
    let visual = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .build();
    let label = player.text("Mini player display", "Visual do miniplayer");
    visual.set_tooltip_text(Some(label));
    visual.update_property(&[gtk::accessible::Property::Label(label)]);
    let popover = gtk::Popover::new();
    popover.add_css_class("player-menu");
    let choices = gtk::Box::new(gtk::Orientation::Vertical, 5);
    let cover_choice = gtk::CheckButton::with_label(player.text("Album cover", "Capa do álbum"));
    cover_choice.set_active(saved.cover || fallback_cover);
    let waveform_choice =
        gtk::CheckButton::with_label(player.text("Audio waveform", "Ondas do áudio"));
    waveform_choice.set_active(saved.waveform);
    waveform_choice.set_sensitive(waveform_enabled);
    let lyrics_choice = gtk::CheckButton::with_label(player.text("Lyrics", "Letra"));
    lyrics_choice.set_active(saved.lyrics);
    lyrics_choice.set_sensitive(lyrics_enabled);
    let styles = gtk::Box::new(gtk::Orientation::Vertical, 3);
    styles.set_margin_start(18);
    styles.update_property(&[gtk::accessible::Property::Label(
        player.text("Waveform style", "Estilo das ondas"),
    )]);
    let style_choices = [
        (
            MiniVisual::Waveform,
            player.text("Waveform line", "Onda em linha"),
        ),
        (
            MiniVisual::Bars,
            player.text("Waveform bars", "Onda em barras"),
        ),
        (
            MiniVisual::Mirror,
            player.text("Mirrored waveform", "Onda espelhada"),
        ),
        (
            MiniVisual::Ring,
            player.text("Pulsing ring", "Anel pulsante"),
        ),
        (MiniVisual::Dots, player.text("LED meter", "Matriz de LEDs")),
        (MiniVisual::Aurora, player.text("Aurora", "Aurora")),
    ]
    .map(|(visual, label)| (visual, gtk::CheckButton::with_label(label)));
    let selected_style = if matches!(
        saved.waveform_style,
        MiniVisual::Bars
            | MiniVisual::Mirror
            | MiniVisual::Ring
            | MiniVisual::Dots
            | MiniVisual::Aurora
    ) {
        saved.waveform_style
    } else {
        MiniVisual::Waveform
    };
    mode.set(selected_style);
    for (visual, choice) in &style_choices {
        choice.set_group(Some(&style_choices[0].1).filter(|first| *first != choice));
        choice.set_active(*visual == selected_style);
        styles.append(choice);
    }

    let update = {
        let info = info.clone();
        let weak = Rc::downgrade(player);
        let cover_choice = cover_choice.downgrade();
        let waveform_choice = waveform_choice.downgrade();
        let lyrics_choice = lyrics_choice.downgrade();
        let styles = styles.downgrade();
        let mode = mode.clone();
        let area = area.downgrade();
        let update = move |persist| {
            let (
                Some(player),
                Some(cover_choice),
                Some(waveform_choice),
                Some(lyrics_choice),
                Some(styles),
                Some(area),
            ) = (
                weak.upgrade(),
                cover_choice.upgrade(),
                waveform_choice.upgrade(),
                lyrics_choice.upgrade(),
                styles.upgrade(),
                area.upgrade(),
            )
            else {
                return;
            };
            let display = MiniDisplay {
                cover: cover_choice.is_active(),
                waveform: waveform_choice.is_active(),
                lyrics: lyrics_choice.is_active(),
                waveform_style: mode.get(),
            };
            let show_waveform = display.waveform && waveform_enabled;
            let show_lyrics = display.lyrics && lyrics_enabled;
            // Lyrics take the middle: the cover becomes a thumbnail in a header instead.
            compact.set_visible(show_lyrics);
            thumb.set_visible(show_lyrics && display.cover);
            info.set_visible(!show_lyrics);
            cover_frame.set_visible(display.cover && !show_lyrics);
            area.set_visible(show_waveform);
            area.set_vexpand(show_waveform && !display.cover && !show_lyrics);
            // The ring needs room to be round; the other styles read well as a strip.
            area.set_content_height(if mode.get() == MiniVisual::Ring {
                150
            } else {
                56
            });
            artwork.set_vexpand(!show_lyrics);
            artwork.set_visible((display.cover && !show_lyrics) || show_waveform);
            lyric_panel.set_visible(show_lyrics);
            visuals.set_visible(display.cover || show_waveform || show_lyrics);
            styles.set_visible(show_waveform);
            if let Some(source) = &player.waveform {
                source.set_enabled(show_waveform);
            }
            area.queue_draw();
            if persist {
                player.state.borrow_mut().mini_display = Some(display);
                player.save();
            }
        };
        Rc::new(update)
    };
    update(false);
    for choice in [&cover_choice, &waveform_choice, &lyrics_choice] {
        let update = update.clone();
        choice.connect_toggled(move |_| update(true));
    }
    for (visual, choice) in style_choices {
        let update = update.clone();
        let mode = mode.clone();
        choice.connect_toggled(move |choice| {
            if choice.is_active() {
                mode.set(visual);
                update(true);
            }
        });
    }
    choices.append(&cover_choice);
    choices.append(&waveform_choice);
    choices.append(&styles);
    choices.append(&lyrics_choice);
    popover.set_child(Some(&choices));
    visual.set_popover(Some(&popover));
    layout.append(&info);

    let position = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let progress = gtk::Scale::builder()
        .orientation(gtk::Orientation::Horizontal)
        .adjustment(&player.progress.adjustment())
        .draw_value(false)
        .hexpand(true)
        .build();
    player
        .progress
        .bind_property("sensitive", &progress, "sensitive")
        .sync_create()
        .build();
    progress.set_tooltip_text(Some(
        player.text("Playback position", "Posição da reprodução"),
    ));
    let weak = Rc::downgrade(player);
    progress.connect_change_value(move |_, _, seconds| {
        if let Some(player) = weak.upgrade() {
            player.seek(seconds);
        }
        glib::Propagation::Proceed
    });
    position.append(&progress);
    let time = gtk::Label::new(None);
    time.add_css_class("playback-clock");
    time.set_xalign(1.0);
    for property in ["label", "width-chars"] {
        player
            .time
            .bind_property(property, &time, property)
            .sync_create()
            .build();
    }
    position.append(&time);
    layout.append(&position);

    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    controls.add_css_class("transport");
    for (icon, label, step) in [
        (
            "media-skip-backward-symbolic",
            player.text("Previous track", "Faixa anterior"),
            -1,
        ),
        (
            "media-playback-start-symbolic",
            player.text("Play", "Tocar"),
            0,
        ),
        (
            "media-skip-forward-symbolic",
            player.text("Next track", "Próxima faixa"),
            1,
        ),
    ] {
        let control = button(icon, label);
        if step == 0 {
            control.add_css_class("play-button");
            for property in ["icon-name", "tooltip-text", "sensitive"] {
                player
                    .toggle
                    .bind_property(property, &control, property)
                    .sync_create()
                    .build();
            }
        }
        let weak = Rc::downgrade(player);
        control.connect_clicked(move |_| {
            if let Some(player) = weak.upgrade() {
                if step == 0 {
                    player.toggle();
                } else {
                    player.step(step);
                }
            }
        });
        controls.append(&control);
    }
    let volume = gtk::Scale::builder()
        .orientation(gtk::Orientation::Horizontal)
        .adjustment(&player.volume.adjustment())
        .draw_value(false)
        .hexpand(true)
        .build();
    volume.set_tooltip_text(Some(player.text("Volume", "Volume")));
    controls.append(&visual);
    controls.append(&volume);
    // The play button is taller than the others; centering keeps the rest round.
    let mut child = controls.first_child();
    while let Some(widget) = child {
        widget.set_valign(gtk::Align::Center);
        child = widget.next_sibling();
    }
    layout.append(&controls);
    window.set_child(Some(&layout));

    let weak = Rc::downgrade(player);
    window.connect_close_request(move |_| {
        if let Some(player) = weak.upgrade() {
            player.close_track_menu();
            if let Some(waveform) = &player.waveform {
                waveform.set_enabled(false);
                waveform.clear();
            }
            player.mini.replace(None);
            player.window.present();
        }
        glib::Propagation::Proceed
    });
    if let Some(source) = player.waveform.clone() {
        let weak = Rc::downgrade(player);
        let last_tick = Cell::new(0);
        area.add_tick_callback(move |area, clock| {
            let Some(player) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let now = clock.frame_time();
            let elapsed = (now - last_tick.replace(now)).clamp(0, 50_000) as f32 / 1_000_000.0;
            let target = if player.playing.get() {
                source.snapshot()
            } else {
                WaveformFrame::default()
            };
            let previous = frame.get();
            let next = smooth_frame(previous, target, elapsed);
            if next != previous {
                frame.set(next);
                area.queue_draw();
            }
            glib::ControlFlow::Continue
        });
    }
    player.mini.replace(Some(window.clone()));
    window.present();
    player.window.set_visible(false);
}

fn plain_lyrics(player: &Player) -> gtk::ScrolledWindow {
    let text = gtk::Label::new(None);
    text.set_wrap(true);
    text.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    text.set_justify(gtk::Justification::Center);
    text.set_valign(gtk::Align::Center);
    text.add_css_class("mini-lyrics");
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(125)
        .child(&text)
        .build();
    let adjustment = scroll.vadjustment().downgrade();
    text.connect_label_notify(move |_| {
        if let Some(adjustment) = adjustment.upgrade() {
            adjustment.set_value(0.0);
        }
    });
    player
        .mini_lyrics
        .bind_property("label", &text, "label")
        .sync_create()
        .build();
    scroll
}

fn smooth_frame(mut frame: WaveformFrame, target: WaveformFrame, elapsed: f32) -> WaveformFrame {
    fn ease(current: f32, target: f32, amount: f32) -> f32 {
        if (current - target).abs() < 0.001 {
            target
        } else {
            current + (target - current) * amount
        }
    }

    let line_speed = 1.0 - (-elapsed / 0.055).exp();
    let rise = 1.0 - (-elapsed / 0.075).exp();
    let fall = 1.0 - (-elapsed / 0.210).exp();
    for i in 0..POINTS {
        frame.line[i] = ease(frame.line[i], target.line[i], line_speed);
        let speed = if target.energy[i] > frame.energy[i] {
            rise
        } else {
            fall
        };
        frame.energy[i] = ease(frame.energy[i], target.energy[i], speed);
    }
    frame
}

fn draw_waveform(
    cr: &gtk::cairo::Context,
    width: f64,
    height: f64,
    frame: &WaveformFrame,
    mode: MiniVisual,
    accent: (f64, f64, f64),
) {
    match mode {
        MiniVisual::Ring => return draw_ring(cr, width, height, frame, accent),
        MiniVisual::Dots => return draw_dots(cr, width, height, frame, accent),
        MiniVisual::Aurora => return draw_aurora(cr, width, height, frame, accent),
        _ => {}
    }
    let middle = height / 2.0;
    let inset = 14.0;
    let width = (width - 2.0 * inset).max(1.0);
    cr.set_source_rgba(accent.0, accent.1, accent.2, 0.16);
    cr.set_line_width(1.0);
    cr.move_to(inset, middle);
    cr.line_to(inset + width, middle);
    let _ = cr.stroke();
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    if mode == MiniVisual::Bars {
        let spacing = width / POINTS as f64;
        for (i, energy) in frame.energy.iter().enumerate() {
            let x = inset + (i as f64 + 0.5) * spacing;
            let level = (f64::from(*energy) * 1.9).tanh();
            let radius = level * height * 0.27;
            cr.set_line_width((spacing * 0.50).clamp(1.0, 7.0));
            cr.set_source_rgba(accent.0, accent.1, accent.2, 0.20 + level * 0.65);
            cr.move_to(x, middle - radius);
            cr.line_to(x, middle + radius);
            let _ = cr.stroke();
        }
        return;
    }

    let mirror = mode == MiniVisual::Mirror;
    let (samples, gain) = if mirror {
        (&frame.energy, 1.9)
    } else {
        (&frame.line, 3.0)
    };
    let points: [(f64, f64); POINTS] = std::array::from_fn(|i| {
        let x = inset + width * i as f64 / (POINTS - 1) as f64;
        let y = middle - (f64::from(samples[i]) * gain).tanh() * height * 0.29;
        (x, y)
    });
    cr.move_to(points[0].0, points[0].1);
    trace_curve(cr, points);
    if mirror {
        let lower = points.into_iter().rev().map(|(x, y)| (x, height - y));
        cr.line_to(points[POINTS - 1].0, height - points[POINTS - 1].1);
        trace_curve(cr, lower);
    } else {
        cr.line_to(inset + width, middle);
        cr.line_to(inset, middle);
    }
    cr.close_path();
    let fill = gtk::cairo::LinearGradient::new(0.0, 0.0, 0.0, height);
    for (offset, alpha) in [(0.0, 0.48), (0.5, 0.08), (1.0, 0.48)] {
        fill.add_color_stop_rgba(offset, accent.0, accent.1, accent.2, alpha);
    }
    let _ = cr.set_source(&fill);
    let _ = cr.fill();

    let traces = if mirror {
        [Some(points), Some(points.map(|(x, y)| (x, height - y)))]
    } else {
        [Some(points), None]
    };
    for trace in traces.into_iter().flatten() {
        cr.move_to(trace[0].0, trace[0].1);
        trace_curve(cr, trace);
    }
    cr.set_source_rgba(accent.0, accent.1, accent.2, 0.80);
    cr.set_line_width(1.7);
    let _ = cr.stroke();
}

/// `#rrggbb` as cairo color channels.
fn parse_hex(color: &str) -> Option<(f64, f64, f64)> {
    let hex = color.strip_prefix('#').filter(|hex| hex.len() == 6)?;
    let channel = |range| {
        u8::from_str_radix(hex.get(range)?, 16)
            .ok()
            .map(|v| f64::from(v) / 255.0)
    };
    Some((channel(0..2)?, channel(2..4)?, channel(4..6)?))
}

/// Seconds on a steady clock, for gentle motion between audio frames.
fn now_seconds() -> f64 {
    gtk::glib::monotonic_time() as f64 / 1_000_000.0
}

fn level(energy: f32) -> f64 {
    (f64::from(energy) * 1.9).tanh()
}

/// Bars radiating from a ring whose size follows the overall loudness; the ring turns slowly.
fn draw_ring(
    cr: &gtk::cairo::Context,
    width: f64,
    height: f64,
    frame: &WaveformFrame,
    accent: (f64, f64, f64),
) {
    let (cx, cy) = (width / 2.0, height / 2.0);
    let size = width.min(height) / 2.0;
    let average = frame.energy.iter().map(|e| level(*e)).sum::<f64>() / POINTS as f64;
    let radius = size * (0.34 + average * 0.10);
    let turn = now_seconds() * 0.25;
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    // Each energy value appears twice, mirrored, so the ring is symmetric.
    let count = POINTS * 2;
    for i in 0..count {
        let energy = frame.energy[if i < POINTS { i } else { count - 1 - i }];
        let reach = level(energy) * size * 0.52 + 2.0;
        let angle = turn + i as f64 / count as f64 * std::f64::consts::TAU;
        let (sin, cos) = angle.sin_cos();
        cr.set_source_rgba(accent.0, accent.1, accent.2, 0.35 + level(energy) * 0.6);
        cr.set_line_width((size * 0.06).clamp(1.5, 4.0));
        cr.move_to(cx + cos * radius, cy + sin * radius);
        cr.line_to(cx + cos * (radius + reach), cy + sin * (radius + reach));
        let _ = cr.stroke();
    }
    cr.set_source_rgba(accent.0, accent.1, accent.2, 0.18 + average * 0.4);
    cr.arc(cx, cy, radius * 0.82, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
}

/// Columns of dots lit from the bottom up, with a peak dot in full color.
fn draw_dots(
    cr: &gtk::cairo::Context,
    width: f64,
    height: f64,
    frame: &WaveformFrame,
    accent: (f64, f64, f64),
) {
    let columns = POINTS;
    let rows = ((height / 9.0).floor() as usize).clamp(4, 14);
    let step_x = width / columns as f64;
    let step_y = height / rows as f64;
    let dot = (step_x.min(step_y) * 0.32).max(1.2);
    for (column, energy) in frame.energy.iter().enumerate() {
        let lit = (level(*energy) * rows as f64).round() as usize;
        let x = (column as f64 + 0.5) * step_x;
        for row in 0..rows {
            let y = height - (row as f64 + 0.5) * step_y;
            let alpha = if row + 1 == lit {
                1.0
            } else if row < lit {
                0.35 + 0.45 * row as f64 / rows as f64
            } else {
                0.08
            };
            cr.set_source_rgba(accent.0, accent.1, accent.2, alpha);
            cr.arc(x, y, dot, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
    }
}

/// Three soft waves, offset in phase and shade, drifting slowly sideways.
fn draw_aurora(
    cr: &gtk::cairo::Context,
    width: f64,
    height: f64,
    frame: &WaveformFrame,
    accent: (f64, f64, f64),
) {
    let time = now_seconds();
    // Lighter and darker shades of the accent for depth.
    let shade = |amount: f64| {
        let mix = |c: f64| {
            if amount >= 0.0 {
                c + (1.0 - c) * amount
            } else {
                c * (1.0 + amount)
            }
        };
        (mix(accent.0), mix(accent.1), mix(accent.2))
    };
    for (layer, (tone, alpha, speed)) in [(-0.35, 0.30, 0.6), (0.0, 0.38, 0.9), (0.45, 0.32, 1.3)]
        .into_iter()
        .enumerate()
    {
        let color = shade(tone);
        let base = height * (0.42 + layer as f64 * 0.12);
        let points: [(f64, f64); POINTS] = std::array::from_fn(|i| {
            let x = width * i as f64 / (POINTS - 1) as f64;
            let sway = (time * speed + i as f64 * 0.45 + layer as f64 * 1.7).sin() * height * 0.06;
            let pulse = level(frame.energy[(i + layer * 5) % POINTS]) * height * 0.30;
            (x, base - pulse - sway)
        });
        cr.move_to(points[0].0, points[0].1);
        trace_curve(cr, points);
        cr.line_to(width, height);
        cr.line_to(0.0, height);
        cr.close_path();
        let fill = gtk::cairo::LinearGradient::new(0.0, base - height * 0.35, 0.0, height);
        fill.add_color_stop_rgba(0.0, color.0, color.1, color.2, alpha);
        fill.add_color_stop_rgba(1.0, color.0, color.1, color.2, 0.0);
        let _ = cr.set_source(&fill);
        let _ = cr.fill();
    }
}

fn trace_curve(cr: &gtk::cairo::Context, points: impl IntoIterator<Item = (f64, f64)>) {
    let mut points = points.into_iter();
    let Some((mut px, mut py)) = points.next() else {
        return;
    };
    for (x, y) in points {
        let mid = (px + x) / 2.0;
        cr.curve_to(mid, py, mid, y, x, y);
        (px, py) = (x, y);
    }
}

fn button(icon: &str, label: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.set_tooltip_text(Some(label));
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button
}
