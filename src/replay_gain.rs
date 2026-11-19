use std::sync::{Arc, Mutex};

use gst::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
pub enum Mode {
    #[default]
    Off,
    Track,
    Album,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(default)]
pub struct Settings {
    pub mode: Mode,
    pub protect_peaks: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Off,
            protect_peaks: true,
        }
    }
}

#[derive(Default)]
struct State {
    settings: Settings,
    track_gain: Option<f64>,
    track_peak: Option<f64>,
    album_gain: Option<f64>,
    album_peak: Option<f64>,
}

impl State {
    fn gain(&self) -> f64 {
        let (gain, peak) = match self.settings.mode {
            Mode::Off => return 1.0,
            Mode::Album if self.album_gain.is_some() => (self.album_gain, self.album_peak),
            Mode::Album | Mode::Track => (self.track_gain, self.track_peak),
        };
        let Some(gain) = gain else { return 1.0 };
        let multiplier = 10_f64.powf(gain.clamp(-40.0, 20.0) / 20.0);
        if self.settings.protect_peaks {
            // Without a known peak, avoid boosting a signal that might already reach full scale.
            multiplier.min(1.0 / peak.unwrap_or(1.0))
        } else {
            multiplier
        }
    }

    fn read_tags(&mut self, list: &gst::TagListRef) {
        use gst::tags::{AlbumGain, AlbumPeak, TrackGain, TrackPeak};
        if let Some(value) = list
            .get::<TrackGain>()
            .map(|tag| tag.get())
            .filter(|value| value.is_finite())
        {
            self.track_gain = Some(value);
        }
        if let Some(value) = list
            .get::<AlbumGain>()
            .map(|tag| tag.get())
            .filter(|value| value.is_finite())
        {
            self.album_gain = Some(value);
        }
        if let Some(value) = list
            .get::<TrackPeak>()
            .map(|tag| tag.get())
            .filter(|value| value.is_finite() && *value > 0.0)
        {
            self.track_peak = Some(value);
        }
        if let Some(value) = list
            .get::<AlbumPeak>()
            .map(|tag| tag.get())
            .filter(|value| value.is_finite() && *value > 0.0)
        {
            self.album_peak = Some(value);
        }
    }
}

pub struct ReplayGain {
    volume: gst::Element,
    state: Arc<Mutex<State>>,
}

impl ReplayGain {
    pub fn attach(volume: gst::Element) -> Option<Self> {
        let pad = volume.static_pad("sink")?;
        let state = Arc::new(Mutex::new(State::default()));
        let shared = state.clone();
        let weak = volume.downgrade();
        pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_, info| {
            let (Some(event), Some(volume)) = (info.event(), weak.upgrade()) else {
                return gst::PadProbeReturn::Ok;
            };
            if let Ok(mut state) = shared.lock() {
                match event.view() {
                    gst::EventView::StreamStart(_) => {
                        *state = State {
                            settings: state.settings,
                            ..State::default()
                        };
                    }
                    gst::EventView::Tag(tag) => state.read_tags(tag.tag()),
                    _ => return gst::PadProbeReturn::Ok,
                }
                volume.set_property("volume", state.gain());
            }
            gst::PadProbeReturn::Ok
        });
        Some(Self { volume, state })
    }

    pub fn apply(&self, settings: Settings) {
        if let Ok(mut state) = self.state.lock() {
            state.settings = settings;
            self.volume.set_property("volume", state.gain());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_uses_album_and_track_tags_and_limits_peak_boost() {
        let mut state = State {
            track_gain: Some(-6.0),
            track_peak: Some(0.8),
            album_gain: Some(6.0),
            album_peak: Some(0.8),
            ..State::default()
        };
        assert_eq!(state.gain(), 1.0);
        state.settings.mode = Mode::Album;
        assert_eq!(state.gain(), 1.25);
        state.album_gain = None;
        assert!((state.gain() - 0.501187).abs() < 1e-6);
        state.track_gain = None;
        assert_eq!(state.gain(), 1.0);
    }

    #[test]
    fn volume_updates_when_the_audio_filter_receives_tags() {
        gst::init().unwrap();
        let pipeline =
            gst::parse::launch("audiotestsrc is-live=true ! volume name=test-volume ! fakesink")
                .unwrap()
                .downcast::<gst::Pipeline>()
                .unwrap();
        let volume = pipeline.by_name("test-volume").unwrap();
        let controller = ReplayGain::attach(volume.clone()).unwrap();
        controller.apply(Settings {
            mode: Mode::Track,
            ..Settings::default()
        });
        pipeline.set_state(gst::State::Playing).unwrap();
        let (result, _, _) = pipeline.state(gst::ClockTime::from_seconds(2));
        assert!(result.is_ok());
        let mut tags = gst::TagList::new();
        tags.get_mut()
            .unwrap()
            .add::<gst::tags::TrackGain>(&-6.0, gst::TagMergeMode::Replace);
        volume
            .static_pad("sink")
            .unwrap()
            .send_event(gst::event::Tag::new(tags));
        let gain = volume.property::<f64>("volume");
        pipeline.set_state(gst::State::Null).unwrap();
        assert!((gain - 0.501187).abs() < 1e-6);
    }
}
