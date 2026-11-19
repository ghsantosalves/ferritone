use gst::prelude::*;
use serde::{Deserialize, Serialize};

pub const BANDS: [&str; 10] = [
    "29 Hz", "59 Hz", "119 Hz", "237 Hz", "474 Hz", "947 Hz", "1.9 kHz", "3.8 kHz", "7.5 kHz",
    "15 kHz",
];

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub gains: [f64; 10],
}

pub const PRESETS: [(&str, &str, [f64; 10]); 6] = [
    ("Flat", "Plano", [0.; 10]),
    ("Rock", "Rock", [4., 3., 2., -1., -2., 0., 2., 3., 4., 3.]),
    (
        "Classical",
        "Clássica",
        [3., 2., 1., 0., 0., 0., 1., 2., 3., 3.],
    ),
    ("Jazz", "Jazz", [3., 2., 1., 1., -1., -1., 0., 1., 2., 2.]),
    (
        "Electronic",
        "Eletrônica",
        [5., 4., 3., 0., -2., 0., 1., 2., 4., 4.],
    ),
    (
        "Vocal",
        "Vocal",
        [-3., -2., -1., 0., 2., 4., 4., 3., 1., 0.],
    ),
];

impl Settings {
    pub fn apply(&self, equalizer: &gst::Element) {
        for (band, gain) in self.gains.iter().enumerate() {
            let gain = if self.enabled {
                gain.clamp(-24.0, 12.0)
            } else {
                0.0
            };
            equalizer.set_property(format!("band{band}").as_str(), gain);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_gains_and_bypasses_them_without_erasing_settings() {
        gst::init().unwrap();
        let pipeline = gst::ElementFactory::make("playbin").build().unwrap();
        let (equalizer, _, _) = crate::waveform::attach(&pipeline);
        let equalizer = equalizer.expect("GStreamer Good Plug-ins are required");
        let mut settings = Settings {
            enabled: true,
            ..Settings::default()
        };
        settings.gains[3] = 5.0;
        settings.apply(&equalizer);
        assert_eq!(equalizer.property::<f64>("band3"), 5.0);
        settings.enabled = false;
        settings.apply(&equalizer);
        assert_eq!(equalizer.property::<f64>("band3"), 0.0);
        assert_eq!(settings.gains[3], 5.0);
    }
}
