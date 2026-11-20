//! Playback decisions that do not depend on GTK or GStreamer, so they can be tested.

use std::time::{Duration, Instant};

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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(result, OnError::GiveUp { resume: 10, several: true });
    }

    #[test]
    fn a_short_queue_stops_once_every_track_failed() {
        let mut failures = Failures::default();
        let (tried, result) = run(&mut failures, 0, 3);
        assert_eq!(tried, 3);
        assert_eq!(result, OnError::GiveUp { resume: 0, several: true });
    }

    #[test]
    fn a_single_failure_at_the_end_of_the_queue_stays_on_that_track() {
        let mut failures = Failures::default();
        assert_eq!(
            failures.failed(4, 5, false),
            OnError::GiveUp { resume: 4, several: false }
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
        assert!(matches!(failures.failed(4, 5, false), OnError::GiveUp { .. }));
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
