//! The sleep timer: fading out and pausing, and the window that sets it.

use super::*;
use playback::SleepTimer;

const SLEEP_MINUTES: [u64; 5] = [15, 30, 45, 60, 90];

impl Player {
    /// Sets the pipeline volume from the slider, lowered while the sleep timer fades out.
    pub(crate) fn apply_volume(&self) {
        let factor = self.sleep.get().volume_factor(Instant::now());
        let volume = self.volume.value() / 100.0 * factor;
        if (volume - self.applied_volume.get()).abs() > 1e-4 {
            self.applied_volume.set(volume);
            self.pipeline.set_property("volume", volume);
        }
    }

    /// Called from `poll`: asks the session not to lock the screen or suspend while music
    /// plays, as chosen in Preferences.
    pub(crate) fn update_inhibit(&self) {
        let flags = {
            let state = self.state.borrow();
            let mut flags = gtk::ApplicationInhibitFlags::empty();
            if !state.allow_screen_lock {
                flags |= gtk::ApplicationInhibitFlags::IDLE;
            }
            if !state.allow_sleep {
                flags |= gtk::ApplicationInhibitFlags::SUSPEND;
            }
            flags
        };
        let wanted = self.playing.get() && !flags.is_empty();
        let cookie = self.inhibit_cookie.get();
        let Some(app) = self.window.application() else {
            return;
        };
        if wanted && cookie == 0 {
            let cookie = app.inhibit(
                Some(&self.window),
                flags,
                Some(self.text("Playing music", "Tocando música")),
            );
            self.inhibit_cookie.set(cookie);
        } else if !wanted && cookie != 0 {
            app.uninhibit(cookie);
            self.inhibit_cookie.set(0);
        }
    }

    /// Applies changed lock or suspend preferences to the current inhibition.
    pub(crate) fn refresh_inhibit(&self) {
        let cookie = self.inhibit_cookie.replace(0);
        if cookie != 0 {
            if let Some(app) = self.window.application() {
                app.uninhibit(cookie);
            }
        }
        self.update_inhibit();
    }

    /// Called from `poll`: fades and pauses when a timed sleep is due.
    pub(crate) fn poll_sleep(&self) {
        let timer = self.sleep.get();
        if !matches!(timer, SleepTimer::At(_)) {
            return;
        }
        if timer.expired(Instant::now()) {
            self.sleep.set(SleepTimer::Off);
            if self.playing.get() && self.pipeline.set_state(gst::State::Paused).is_ok() {
                self.playing.set(false);
                self.update_position();
                self.update_transport();
                self.save();
            }
            self.apply_volume();
            self.message(self.text(
                "The sleep timer paused playback.",
                "O timer pausou a reprodução.",
            ));
        } else {
            self.apply_volume();
        }
    }

    /// Called at the end of a track when the timer is set to stop there.
    /// Moves to the next track without playing it, so Play continues from there.
    pub(crate) fn sleep_at_track_end(self: &Rc<Self>) {
        self.sleep.set(SleepTimer::Off);
        let next = self.state.borrow().next_index(1, true);
        if let Some(index) = next {
            self.state.borrow_mut().current = index;
            self.load_current_track();
        }
        self.stop();
        self.update_transport();
        self.message(self.text(
            "The sleep timer paused playback.",
            "O timer pausou a reprodução.",
        ));
    }

    fn set_sleep(&self, timer: SleepTimer) {
        self.sleep.set(timer);
        self.apply_volume();
        let message = match timer {
            SleepTimer::Off => String::new(),
            SleepTimer::EndOfTrack => self
                .text(
                    "Playback will pause at the end of this track.",
                    "A reprodução vai pausar no fim desta faixa.",
                )
                .to_owned(),
            SleepTimer::At(_) => self.sleep_status(),
        };
        self.message(&message);
    }

    fn sleep_status(&self) -> String {
        match self.sleep.get() {
            SleepTimer::Off => self
                .text("The sleep timer is off.", "O timer está desligado.")
                .to_owned(),
            SleepTimer::EndOfTrack => self
                .text(
                    "Playback will pause at the end of this track.",
                    "A reprodução vai pausar no fim desta faixa.",
                )
                .to_owned(),
            SleepTimer::At(_) => {
                let minutes = self.sleep.get().minutes_left(Instant::now()).unwrap_or(0);
                self.text(
                    "Playback will pause in {} min.",
                    "A reprodução vai pausar em {} min.",
                )
                .replace("{}", &minutes.to_string())
            }
        }
    }

    pub(crate) fn show_sleep_timer(self: &Rc<Self>) {
        let window = gtk::Window::builder()
            .title(self.text("Sleep timer", "Timer para desligar"))
            .transient_for(&self.window)
            .modal(true)
            .resizable(false)
            .default_width(320)
            .build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.add_css_class("page-actions");
        content.set_margin_top(18);
        content.set_margin_bottom(18);
        content.set_margin_start(18);
        content.set_margin_end(18);
        let status = gtk::Label::new(Some(&self.sleep_status()));
        status.set_wrap(true);
        status.set_xalign(0.0);
        status.add_css_class("dim-label");
        status.set_margin_bottom(6);
        content.append(&status);

        // `None` minutes means the end of the track; the timer starts when an option is chosen.
        let mut options: Vec<(String, Option<Option<u64>>)> = SLEEP_MINUTES
            .iter()
            .map(|minutes| {
                (
                    self.text("{} minutes", "{} minutos")
                        .replace("{}", &minutes.to_string()),
                    Some(Some(*minutes)),
                )
            })
            .collect();
        options.push((
            self.text("End of current track", "Fim da faixa atual")
                .to_owned(),
            Some(None),
        ));
        options.push((self.text("Off", "Desligado").to_owned(), None));
        for (label, choice) in options {
            let close = window.downgrade();
            content.append(&button(&label, self, move |player| {
                player.set_sleep(match choice {
                    Some(Some(minutes)) => SleepTimer::after(Instant::now(), minutes),
                    Some(None) => SleepTimer::EndOfTrack,
                    None => SleepTimer::Off,
                });
                if let Some(window) = close.upgrade() {
                    window.close();
                }
            }));
        }
        window.set_child(Some(&content));
        window.present();
    }
}
