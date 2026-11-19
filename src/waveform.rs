use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use gst::prelude::*;

pub const POINTS: usize = 24;

#[derive(Clone, Copy, Default, PartialEq)]
pub struct WaveformFrame {
    pub line: [f32; POINTS],
    pub energy: [f32; POINTS],
}

#[derive(Clone)]
pub struct Waveform(Arc<WaveformState>);

struct WaveformState {
    enabled: AtomicBool,
    frame: Mutex<WaveformFrame>,
}

impl Waveform {
    pub fn set_enabled(&self, enabled: bool) {
        self.0.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn clear(&self) {
        if let Ok(mut frame) = self.0.frame.lock() {
            *frame = WaveformFrame::default();
        }
    }

    pub fn snapshot(&self) -> WaveformFrame {
        self.0
            .frame
            .lock()
            .map_or_else(|_| WaveformFrame::default(), |frame| *frame)
    }

    fn observe(&self, pad: &gst::Pad, info: &gst::PadProbeInfo<'_>) {
        if !self.0.enabled.load(Ordering::Relaxed) {
            return;
        }
        let Some(caps) = pad.current_caps() else {
            return;
        };
        let Some(format) = caps.structure(0) else {
            return;
        };
        let Ok(channels) = format.get::<i32>("channels") else {
            return;
        };
        let Some(buffer) = info.buffer() else { return };
        let Ok(data) = buffer.map_readable() else {
            return;
        };
        if channels <= 0 {
            return;
        }
        let stride = channels as usize * size_of::<f32>();
        let frames = data.len() / stride;
        if frames == 0 {
            return;
        }
        if let Ok(mut frame) = self.0.frame.try_lock() {
            if !self.0.enabled.load(Ordering::Relaxed) {
                return;
            }
            // Average short time slices. RMS levels react to sustained sound instead of isolated peaks.
            for index in 0..POINTS {
                let start = index * frames / POINTS;
                let end = ((index + 1) * frames / POINTS).max(start + 1);
                let step = ((end - start) / 32).max(1);
                let mut sum = 0.0;
                let mut squares = 0.0;
                let mut count = 0;
                for channel_frame in data[start * stride..end * stride]
                    .chunks_exact(stride)
                    .step_by(step)
                {
                    let mut encoded = [0; 4];
                    encoded.copy_from_slice(&channel_frame[..4]);
                    let value = f32::from_le_bytes(encoded);
                    let sample = if value.is_finite() {
                        value.clamp(-1.0, 1.0)
                    } else {
                        0.0
                    };
                    sum += sample;
                    squares += sample * sample;
                    count += 1;
                }
                frame.line[index] = sum / count as f32;
                frame.energy[index] = (squares / count as f32).sqrt();
            }
        }
    }
}

pub fn attach(
    pipeline: &gst::Element,
) -> (
    Option<gst::Element>,
    Option<Waveform>,
    Option<crate::replay_gain::ReplayGain>,
) {
    let equalizer_available = gst::ElementFactory::find("equalizer-10bands").is_some();
    let mut description = String::from(
        "audioconvert ! audio/x-raw,format=F32LE,layout=interleaved ! identity name=waveform",
    );
    if gst::ElementFactory::find("volume").is_some() {
        description.push_str(" ! volume name=replay-gain");
    }
    if equalizer_available {
        description.push_str(" ! equalizer-10bands name=equalizer");
    }
    pipeline.set_property_from_str("flags", "audio+soft-volume");
    let Ok(filter) = gst::parse::bin_from_description(&description, true) else {
        let equalizer = gst::ElementFactory::make("equalizer-10bands").build().ok();
        if let Some(equalizer) = &equalizer {
            pipeline.set_property_from_str("flags", "audio+soft-volume+force-filters");
            pipeline.set_property("audio-filter", equalizer);
        }
        return (equalizer, None, None);
    };
    let equalizer = filter.by_name("equalizer");
    let replay_gain = filter
        .by_name("replay-gain")
        .and_then(crate::replay_gain::ReplayGain::attach);
    let waveform = Waveform(Arc::new(WaveformState {
        enabled: AtomicBool::new(false),
        frame: Mutex::new(WaveformFrame::default()),
    }));
    let Some(pad) = filter
        .by_name("waveform")
        .and_then(|element| element.static_pad("src"))
    else {
        return (None, None, None);
    };
    let probe = waveform.clone();
    pad.add_probe(gst::PadProbeType::BUFFER, move |pad, info| {
        probe.observe(pad, info);
        gst::PadProbeReturn::Ok
    });
    pipeline.set_property_from_str("flags", "audio+soft-volume+force-filters");
    pipeline.set_property("audio-filter", &filter);
    (equalizer, Some(waveform), replay_gain)
}
