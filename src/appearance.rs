//! Follows the desktop's light or dark preference and accent color through the settings portal,
//! which works the same inside and outside Flatpak and reports changes as they happen.

use super::*;

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const NAMESPACE: &str = "org.freedesktop.appearance";

/// GNOME's accent colors, to name the color the portal reports as RGB.
const ACCENTS: [(&str, [f64; 3]); 9] = [
    ("blue", [0.208, 0.518, 0.894]),
    ("teal", [0.129, 0.565, 0.643]),
    ("green", [0.227, 0.580, 0.290]),
    ("yellow", [0.784, 0.533, 0.0]),
    ("orange", [0.929, 0.357, 0.0]),
    ("red", [0.902, 0.176, 0.259]),
    ("pink", [0.835, 0.380, 0.600]),
    ("purple", [0.569, 0.255, 0.675]),
    ("slate", [0.435, 0.514, 0.588]),
];

/// The portal's color scheme: 1 prefers dark, 2 prefers light, anything else has no preference.
pub fn scheme_name(value: u32) -> &'static str {
    match value {
        1 => "prefer-dark",
        2 => "prefer-light",
        _ => "default",
    }
}

/// The GNOME accent closest to an RGB color, or `None` outside the valid range.
pub fn accent_name(rgb: [f64; 3]) -> Option<&'static str> {
    if rgb.iter().any(|channel| !(0.0..=1.0).contains(channel)) {
        return None;
    }
    ACCENTS
        .iter()
        .min_by(|(_, a), (_, b)| {
            let distance = |color: &[f64; 3]| -> f64 {
                color.iter().zip(rgb).map(|(c, v)| (c - v).powi(2)).sum()
            };
            distance(a).total_cmp(&distance(b))
        })
        .map(|(name, _)| *name)
}

/// Unwraps the `v` (or `(v)`, from the older `Read` method) a portal setting arrives in.
fn unwrap_value(mut value: glib::Variant) -> glib::Variant {
    while let Some(inner) = value.as_variant() {
        value = inner;
    }
    if value.is_container() && value.n_children() == 1 && value.type_().as_str() != "(ddd)" {
        value = value.child_value(0);
        while let Some(inner) = value.as_variant() {
            value = inner;
        }
    }
    value
}

/// Reads a setting without blocking the window: right after login the portal can take a few
/// seconds to start, and a blocking call would freeze Ferritone while it does.
async fn read(connection: &gio::DBusConnection, key: &str) -> Option<glib::Variant> {
    for method in ["ReadOne", "Read"] {
        let reply = connection
            .call_future(
                Some(PORTAL),
                PATH,
                SETTINGS,
                method,
                Some(&(NAMESPACE, key).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                5000,
            )
            .await;
        if let Ok(reply) = reply {
            return Some(unwrap_value(reply));
        }
    }
    None
}

impl Player {
    /// The desktop's color scheme, from the portal when it answers, else from GSettings.
    pub(crate) fn desktop_color_scheme(&self) -> Option<String> {
        if let Some(value) = self.portal_color_scheme.get() {
            return Some(scheme_name(value).to_owned());
        }
        self.desktop_appearance
            .as_ref()
            .map(|appearance| appearance.string("color-scheme").to_string())
    }

    /// The desktop's accent color name, from the portal when it answers, else from GSettings.
    pub(crate) fn desktop_accent(&self) -> Option<String> {
        if let Some(name) = self.portal_accent.get() {
            return Some(name.to_owned());
        }
        self.desktop_appearance
            .as_ref()
            .filter(|settings| {
                settings
                    .settings_schema()
                    .is_some_and(|schema| schema.has_key("accent-color"))
            })
            .map(|settings| settings.string("accent-color").to_string())
    }

    /// Reads the desktop appearance in the background and re-applies the theme whenever it
    /// changes. Until the portal answers, the theme comes from GSettings.
    pub(crate) fn watch_desktop_appearance(self: &Rc<Self>) {
        let Some(connection) = self
            .window
            .application()
            .and_then(|app| app.dbus_connection())
        else {
            return;
        };
        let weak = Rc::downgrade(self);
        let reader = connection.clone();
        glib::spawn_future_local(async move {
            let scheme = read(&reader, "color-scheme")
                .await
                .and_then(|value| value.get::<u32>());
            let accent = read(&reader, "accent-color")
                .await
                .and_then(|value| value.get::<(f64, f64, f64)>())
                .and_then(|rgb| accent_name([rgb.0, rgb.1, rgb.2]));
            let Some(player) = weak.upgrade() else { return };
            let scheme_changed = player.portal_color_scheme.replace(scheme) != scheme;
            let accent_changed = player.portal_accent.replace(accent) != accent;
            if scheme_changed || accent_changed {
                player.apply_theme();
            }
        });
        let weak = Rc::downgrade(self);
        #[allow(deprecated)]
        connection.signal_subscribe(
            Some(PORTAL),
            Some(SETTINGS),
            Some("SettingChanged"),
            Some(PATH),
            Some(NAMESPACE),
            gio::DBusSignalFlags::NONE,
            move |_, _, _, _, _, parameters| {
                let Some(player) = weak.upgrade() else { return };
                let (Some(key), Some(value)) = (
                    parameters
                        .try_child_value(1)
                        .and_then(|key| key.str().map(str::to_owned)),
                    parameters.try_child_value(2).map(unwrap_value),
                ) else {
                    return;
                };
                match key.as_str() {
                    "color-scheme" => player.portal_color_scheme.set(value.get::<u32>()),
                    "accent-color" => player.portal_accent.set(
                        value
                            .get::<(f64, f64, f64)>()
                            .and_then(|rgb| accent_name([rgb.0, rgb.1, rgb.2])),
                    ),
                    _ => return,
                }
                player.apply_theme();
            },
        );
        self.desktop_connection.replace(Some(connection));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_values_map_to_desktop_names() {
        assert_eq!(scheme_name(1), "prefer-dark");
        assert_eq!(scheme_name(2), "prefer-light");
        assert_eq!(scheme_name(0), "default");
        assert_eq!(accent_name([0.2078, 0.5176, 0.8941]), Some("blue"));
        assert_eq!(accent_name([0.93, 0.36, 0.01]), Some("orange"));
        assert_eq!(accent_name([2.0, 0.0, 0.0]), None);
    }

    #[test]
    fn portal_replies_are_unwrapped_from_their_variants() {
        let read_one = 1u32.to_variant().to_variant();
        assert_eq!(unwrap_value(read_one).get::<u32>(), Some(1));
        let read = (1u32.to_variant().to_variant(),).to_variant();
        assert_eq!(unwrap_value(read).get::<u32>(), Some(1));
        let rgb = (0.2f64, 0.5f64, 0.9f64).to_variant().to_variant();
        assert_eq!(
            unwrap_value(rgb).get::<(f64, f64, f64)>(),
            Some((0.2, 0.5, 0.9))
        );
    }
}
