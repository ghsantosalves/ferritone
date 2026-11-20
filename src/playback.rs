//! Playback decisions that do not depend on GTK or GStreamer, so they can be tested.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// Stop skipping after this many tracks in a row fail, so a broken audio setup does not walk the queue.
pub const MAX_SKIPS: usize = 5;
/// Playing this long without an error proves the track works and ends a failure run.
pub const PROGRESS_RESET_MS: u64 = 1000;
/// How often the resume point is saved while playing.
pub const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(30);

/// What to do after a track fails to play.
#[derive(Debug, PartialEq)]
pub enum OnError {
    /// Try the next track in the queue.
    Skip,
    /// Stop playback and move the resume point to `resume`, the first track that failed.
    GiveUp { resume: usize, several: bool },
}

/// Counts consecutive tracks that failed to play.
#[derive(Default)]
pub struct Failures {
    in_row: usize,
    first: usize,
}

impl Failures {
    /// Playback of a new track started, by the user or by the end of the previous track.
    pub fn started(&mut self) {
        self.in_row = 0;
    }

    /// The current track has played for a while, so it works.
    pub fn progressed(&mut self, position_ms: u64) {
        if position_ms >= PROGRESS_RESET_MS {
            self.in_row = 0;
        }
    }

    /// The track at `current` failed. `has_next` says whether the queue has a track after it.
    /// Call `skipped` after starting the next track, since starting resets the count.
    pub fn failed(&mut self, current: usize, queue_len: usize, has_next: bool) -> OnError {
        self.in_row += 1;
        if self.in_row == 1 {
            self.first = current;
        }
        if has_next && self.in_row < queue_len.min(MAX_SKIPS) {
            OnError::Skip
        } else {
            let several = self.in_row > 1;
            let resume = if several { self.first } else { current };
            self.in_row = 0;
            OnError::GiveUp { resume, several }
        }
    }

    /// Keep the count across the `started` call that skipping makes.
    pub fn skipped(&mut self, count: usize) {
        self.in_row = count;
    }

    pub fn count(&self) -> usize {
        self.in_row
    }
}

/// Decides when the resume point is due to be saved while playing.
pub struct Autosave {
    last: Instant,
}

impl Autosave {
    pub fn new(now: Instant) -> Self {
        Self { last: now }
    }

    /// Any save, automatic or not, restarts the interval.
    pub fn saved(&mut self, now: Instant) {
        self.last = now;
    }

    pub fn due(&self, now: Instant, playing: bool) -> bool {
        playing && now.duration_since(self.last) >= AUTOSAVE_INTERVAL
    }
}

/// The volume fades out over this long before the sleep timer pauses playback.
pub const SLEEP_FADE: Duration = Duration::from_secs(30);

/// Pauses playback after a while or at the end of the current track.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SleepTimer {
    #[default]
    Off,
    At(Instant),
    EndOfTrack,
}

impl SleepTimer {
    pub fn after(now: Instant, minutes: u64) -> Self {
        Self::At(now + Duration::from_secs(minutes * 60))
    }

    /// Whether a timed sleep is due now.
    pub fn expired(self, now: Instant) -> bool {
        matches!(self, Self::At(deadline) if now >= deadline)
    }

    /// How much of the chosen volume to keep: 1 until the fade starts, then down to 0.
    pub fn volume_factor(self, now: Instant) -> f64 {
        let Self::At(deadline) = self else { return 1.0 };
        let left = deadline.saturating_duration_since(now);
        (left.as_secs_f64() / SLEEP_FADE.as_secs_f64()).clamp(0.0, 1.0)
    }

    /// Whole minutes left, rounded up, for display.
    pub fn minutes_left(self, now: Instant) -> Option<u64> {
        let Self::At(deadline) = self else {
            return None;
        };
        Some(
            deadline
                .saturating_duration_since(now)
                .as_secs()
                .div_ceil(60),
        )
    }
}

/// Hands the next track to GStreamer before the current one ends, for playback without gaps.
///
/// The UI thread plans the next track; GStreamer's streaming thread takes it when the current
/// track is about to finish; the UI thread then learns of the switch when the new stream starts.
#[derive(Default)]
pub struct Handoff {
    next: Option<(usize, PathBuf)>,
    handed_off: Option<(usize, PathBuf)>,
}

impl Handoff {
    /// The track that should follow the current one, or `None` to let playback end.
    pub fn plan(&mut self, next: Option<(usize, PathBuf)>) {
        self.next = next;
    }

    /// Called from the streaming thread: returns the file to queue in the pipeline.
    pub fn take(&mut self) -> Option<PathBuf> {
        let next = self.next.take()?;
        let path = next.1.clone();
        self.handed_off = Some(next);
        Some(path)
    }

    /// The track that GStreamer moved to on its own, if any, once its stream has started.
    pub fn switched(&mut self) -> Option<(usize, PathBuf)> {
        self.handed_off.take()
    }

    /// Playback was started or stopped by hand, so any planned or pending switch is void.
    pub fn cancel(&mut self) {
        self.next = None;
        self.handed_off = None;
    }
}

/// Finds the queue entry for a track that was handed off, even if the queue changed since.
pub fn resolve(queue: &[PathBuf], index: usize, track: &Path) -> Option<usize> {
    if queue.get(index).map(PathBuf::as_path) == Some(track) {
        Some(index)
    } else {
        queue.iter().position(|entry| entry == track)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sleep_timer_fades_out_then_expires() {
        let start = Instant::now();
        let timer = SleepTimer::after(start, 30);
        assert_eq!(timer.minutes_left(start), Some(30));
        assert_eq!(
            timer.minutes_left(start + Duration::from_secs(61)),
            Some(29)
        );
        let fade_start = start + Duration::from_secs(30 * 60) - SLEEP_FADE;
        assert_eq!(
            timer.volume_factor(fade_start - Duration::from_secs(1)),
            1.0
        );
        let halfway = timer.volume_factor(fade_start + SLEEP_FADE / 2);
        assert!((halfway - 0.5).abs() < 1e-6);
        assert!(!timer.expired(fade_start));
        assert!(timer.expired(start + Duration::from_secs(30 * 60)));
        assert_eq!(
            timer.volume_factor(start + Duration::from_secs(31 * 60)),
            0.0
        );
    }

    #[test]
    fn sleep_at_end_of_track_never_fades_or_expires_by_time() {
        let now = Instant::now();
        for timer in [SleepTimer::Off, SleepTimer::EndOfTrack] {
            assert!(!timer.expired(now + Duration::from_secs(86_400)));
            assert_eq!(timer.volume_factor(now), 1.0);
            assert_eq!(timer.minutes_left(now), None);
        }
    }

    #[test]
    fn handoff_reports_the_switch_once() {
        let mut handoff = Handoff::default();
        assert_eq!(handoff.take(), None);
        handoff.plan(Some((3, PathBuf::from("next.flac"))));
        assert_eq!(handoff.switched(), None);
        assert_eq!(handoff.take(), Some(PathBuf::from("next.flac")));
        // The UI keeps planning from the old track until it hears about the switch.
        handoff.plan(Some((3, PathBuf::from("next.flac"))));
        assert_eq!(handoff.switched(), Some((3, PathBuf::from("next.flac"))));
        assert_eq!(handoff.switched(), None);
    }

    #[test]
    fn starting_by_hand_voids_a_pending_switch() {
        let mut handoff = Handoff::default();
        handoff.plan(Some((1, PathBuf::from("b"))));
        handoff.take();
        handoff.cancel();
        assert_eq!(handoff.switched(), None);
        assert_eq!(handoff.take(), None);
    }

    #[test]
    fn the_end_of_the_queue_hands_off_nothing() {
        let mut handoff = Handoff::default();
        handoff.plan(None);
        assert_eq!(handoff.take(), None);
        assert_eq!(handoff.switched(), None);
    }

    #[test]
    fn resolve_follows_the_track_when_the_queue_changed() {
        let queue: Vec<_> = ["a", "b", "c"].into_iter().map(PathBuf::from).collect();
        assert_eq!(resolve(&queue, 1, Path::new("b")), Some(1));
        // A row was removed above it between the handoff and the switch.
        assert_eq!(resolve(&queue, 2, Path::new("b")), Some(1));
        assert_eq!(resolve(&queue, 0, Path::new("gone")), None);
    }

    /// Feeds failures one by one, like the player does, until it gives up.
    fn run(failures: &mut Failures, start: usize, queue_len: usize) -> (usize, OnError) {
        let mut current = start;
        let mut tried = 0;
        loop {
            tried += 1;
            let has_next = current + 1 < queue_len;
            match failures.failed(current, queue_len, has_next) {
                OnError::Skip => {
                    let count = failures.count();
                    failures.started();
                    failures.skipped(count);
                    current += 1;
                }
                give_up => return (tried, give_up),
            }
        }
    }

    #[test]
    fn a_broken_audio_setup_stops_after_a_few_tracks_and_keeps_the_first_one() {
        let mut failures = Failures::default();
        let (tried, result) = run(&mut failures, 10, 2000);
        assert_eq!(tried, MAX_SKIPS);
        assert_eq!(
            result,
            OnError::GiveUp {
                resume: 10,
                several: true
            }
        );
    }

    #[test]
    fn a_short_queue_stops_once_every_track_failed() {
        let mut failures = Failures::default();
        let (tried, result) = run(&mut failures, 0, 3);
        assert_eq!(tried, 3);
        assert_eq!(
            result,
            OnError::GiveUp {
                resume: 0,
                several: true
            }
        );
    }

    #[test]
    fn a_single_failure_at_the_end_of_the_queue_stays_on_that_track() {
        let mut failures = Failures::default();
        assert_eq!(
            failures.failed(4, 5, false),
            OnError::GiveUp {
                resume: 4,
                several: false
            }
        );
    }

    #[test]
    fn a_track_that_plays_ends_the_failure_run() {
        let mut failures = Failures::default();
        assert_eq!(failures.failed(0, 100, true), OnError::Skip);
        failures.started();
        failures.skipped(1);
        failures.progressed(PROGRESS_RESET_MS - 1);
        assert_eq!(failures.count(), 1);
        failures.progressed(PROGRESS_RESET_MS);
        assert_eq!(failures.count(), 0);
    }

    #[test]
    fn errors_after_preroll_still_add_up() {
        // A pipeline that prerolls and then fails never reports progress, so the count keeps growing.
        let mut failures = Failures::default();
        let (tried, _) = run(&mut failures, 0, 2000);
        assert_eq!(tried, MAX_SKIPS);
    }

    #[test]
    fn a_new_queue_does_not_inherit_failures() {
        let mut failures = Failures::default();
        assert!(matches!(
            failures.failed(4, 5, false),
            OnError::GiveUp { .. }
        ));
        // The user opens a two-track folder; its first track fails, the second is still tried.
        failures.started();
        assert_eq!(failures.failed(0, 2, true), OnError::Skip);
    }

    #[test]
    fn autosave_waits_a_full_interval_after_any_save() {
        let start = Instant::now();
        let mut autosave = Autosave::new(start);
        let later = start + AUTOSAVE_INTERVAL;
        assert!(autosave.due(later, true));
        assert!(!autosave.due(later, false));
        // Pressing play saves; the next poll must not save again right away.
        autosave.saved(later);
        assert!(!autosave.due(later + Duration::from_millis(200), true));
        assert!(autosave.due(later + AUTOSAVE_INTERVAL, true));
    }
}
