use std::{
    cell::RefCell,
    collections::BTreeMap,
    hash::{DefaultHasher, Hash, Hasher},
    io::Write,
    rc::Rc,
};

use glib::variant::ObjectPath;
use gst::prelude::*;
use gtk::{gdk, gio, glib, prelude::*};

use crate::{
    APP_ID, APP_NAME, Import, Player,
    library::{Repeat, supported, title},
};

const BUS_NAME: &str = "org.mpris.MediaPlayer2.Ferritone";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";
type Properties = BTreeMap<&'static str, glib::Variant>;

const INTERFACES: &str = r#"<node>
  <interface name="org.mpris.MediaPlayer2">
    <method name="Raise"/>
    <method name="Quit"/>
    <property name="CanQuit" type="b" access="read"/>
    <property name="CanRaise" type="b" access="read"/>
    <property name="HasTrackList" type="b" access="read"/>
    <property name="Identity" type="s" access="read"/>
    <property name="DesktopEntry" type="s" access="read"/>
    <property name="SupportedUriSchemes" type="as" access="read"/>
    <property name="SupportedMimeTypes" type="as" access="read"/>
  </interface>
  <interface name="org.mpris.MediaPlayer2.Player">
    <method name="Next"/>
    <method name="Previous"/>
    <method name="Pause"/>
    <method name="PlayPause"/>
    <method name="Stop"/>
    <method name="Play"/>
    <method name="Seek"><arg direction="in" type="x" name="Offset"/></method>
    <method name="SetPosition">
      <arg direction="in" type="o" name="TrackId"/>
      <arg direction="in" type="x" name="Position"/>
    </method>
    <method name="OpenUri"><arg direction="in" type="s" name="Uri"/></method>
    <signal name="Seeked"><arg type="x" name="Position"/></signal>
    <property name="PlaybackStatus" type="s" access="read"/>
    <property name="LoopStatus" type="s" access="readwrite"/>
    <property name="Rate" type="d" access="readwrite"/>
    <property name="Shuffle" type="b" access="readwrite"/>
    <property name="Metadata" type="a{sv}" access="read"/>
    <property name="Volume" type="d" access="readwrite"/>
    <property name="Position" type="x" access="read">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="false"/>
    </property>
    <property name="MinimumRate" type="d" access="read"/>
    <property name="MaximumRate" type="d" access="read"/>
    <property name="CanGoNext" type="b" access="read"/>
    <property name="CanGoPrevious" type="b" access="read"/>
    <property name="CanPlay" type="b" access="read"/>
    <property name="CanPause" type="b" access="read"/>
    <property name="CanSeek" type="b" access="read"/>
    <property name="CanControl" type="b" access="read"/>
  </interface>
</node>"#;

#[derive(Default)]
struct Artwork {
    texture: Option<gdk::Texture>,
    file: Option<tempfile::NamedTempFile>,
}

impl Artwork {
    fn update(&mut self, texture: Option<gdk::Texture>) {
        if texture == self.texture {
            return;
        }
        self.file = texture.as_ref().and_then(|texture| {
            let mut file = tempfile::Builder::new()
                .prefix("ferritone-cover-")
                .suffix(".png")
                .tempfile_in(glib::user_runtime_dir())
                .ok()?;
            file.write_all(&texture.save_to_png_bytes()).ok()?;
            Some(file)
        });
        self.texture = texture;
    }
}

pub struct MediaSession {
    connection: gio::DBusConnection,
    registrations: Vec<gio::RegistrationId>,
    name: Option<gio::OwnerId>,
    properties: RefCell<Properties>,
    artwork: RefCell<Artwork>,
}

impl MediaSession {
    pub fn attach(player: &Rc<Player>) -> Option<Self> {
        let connection = player.window.application()?.dbus_connection()?;
        let description = gio::DBusNodeInfo::for_xml(INTERFACES).ok()?;
        let mut session = Self {
            connection,
            registrations: Vec::new(),
            name: None,
            properties: RefCell::default(),
            artwork: RefCell::default(),
        };
        let base = description.lookup_interface("org.mpris.MediaPlayer2")?;
        let weak = Rc::downgrade(player);
        session.registrations.push(
            session
                .connection
                .register_object(PATH, &base)
                .method_call(move |_, _, _, _, method, _, invocation| {
                    if method == "Raise" {
                        if let Some(player) = weak.upgrade() {
                            if let Some(window) = player.mini.borrow().as_ref() {
                                window.present();
                            } else {
                                player.window.present();
                            }
                        }
                    }
                    invocation.return_value(None);
                })
                .property(|_, _, _, _, property| match property {
                    "CanRaise" => true.to_variant(),
                    "CanQuit" | "HasTrackList" => false.to_variant(),
                    "Identity" => APP_NAME.to_variant(),
                    "DesktopEntry" => APP_ID.to_variant(),
                    "SupportedUriSchemes" => ["file"].to_variant(),
                    "SupportedMimeTypes" => Vec::<String>::new().to_variant(),
                    _ => ().to_variant(),
                })
                .build()
                .ok()?,
        );
        let interface = description.lookup_interface(PLAYER)?;
        let call_player = Rc::downgrade(player);
        let read_player = call_player.clone();
        let write_player = call_player.clone();
        session.registrations.push(
            session
                .connection
                .register_object(PATH, &interface)
                .method_call(move |_, _, _, _, method, args, invocation| {
                    if let Some(player) = call_player.upgrade() {
                        player_command(&player, method, &args);
                    }
                    invocation.return_value(None);
                })
                .property(move |_, _, _, _, property| {
                    read_player
                        .upgrade()
                        .and_then(|player| {
                            if property == "Position" {
                                return Some(position(&player).to_variant());
                            }
                            player
                                .media_session
                                .borrow()
                                .as_ref()?
                                .properties
                                .borrow()
                                .get(property)
                                .cloned()
                        })
                        .unwrap_or_else(|| ().to_variant())
                })
                .set_property(move |_, _, _, _, property, value| {
                    write_player
                        .upgrade()
                        .is_some_and(|player| set_property(&player, property, value))
                })
                .build()
                .ok()?,
        );
        session.publish(player);
        session.name = Some(gio::bus_own_name_on_connection(
            &session.connection,
            BUS_NAME,
            gio::BusNameOwnerFlags::DO_NOT_QUEUE,
            |_, _| {},
            |_, _| {},
        ));
        Some(session)
    }

    pub fn publish(&self, player: &Player) {
        let mut artwork = self.artwork.borrow_mut();
        artwork.update(player.cover.borrow().clone());
        let art_url = artwork
            .file
            .as_ref()
            .map(|file| gio::File::for_path(file.path()).uri());
        let next = properties(player, art_url.as_deref());
        let mut previous = self.properties.borrow_mut();
        let changed: Properties = next
            .iter()
            .filter(|(key, value)| previous.get(*key) != Some(*value))
            .map(|(&key, value)| (key, value.clone()))
            .collect();
        *previous = next;
        if !changed.is_empty() {
            let _ = self.connection.emit_signal(
                None,
                PATH,
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
                Some(&(PLAYER, changed, Vec::<String>::new()).to_variant()),
            );
        }
    }

    pub fn seeked(&self, micros: i64) {
        let _ = self.connection.emit_signal(
            None,
            PATH,
            PLAYER,
            "Seeked",
            Some(&(micros,).to_variant()),
        );
    }
}

impl Drop for MediaSession {
    fn drop(&mut self) {
        if let Some(name) = self.name.take() {
            gio::bus_unown_name(name);
        }
        for registration in self.registrations.drain(..) {
            let _ = self.connection.unregister_object(registration);
        }
    }
}

fn properties(player: &Player, art_url: Option<&str>) -> Properties {
    let state = player.state.borrow();
    let info = player.info.borrow();
    let mut metadata = Properties::new();
    let track = state.queue.get(state.current);
    let duration = duration(player);
    if let Some(path) = track {
        let tag = info.get(path);
        let name = tag
            .and_then(|tag| tag.title.as_deref())
            .map_or_else(|| title(path), str::to_owned);
        metadata.insert("mpris:trackid", track_id(state.current, path).to_variant());
        metadata.insert("xesam:title", name.to_variant());
        if let Some(artist) = tag.and_then(|tag| tag.artist.as_deref()) {
            metadata.insert("xesam:artist", [artist].to_variant());
        }
        if let Some(album) = tag.and_then(|tag| tag.album.as_deref()) {
            metadata.insert("xesam:album", album.to_variant());
        }
        let length = (duration > 0).then_some(duration).or_else(|| {
            tag.and_then(|tag| tag.duration_ms)
                .and_then(|ms| i64::try_from(ms).ok())
                .map(|ms| ms.saturating_mul(1000))
        });
        if let Some(length) = length {
            metadata.insert("mpris:length", length.to_variant());
        }
        if let Some(url) = art_url {
            metadata.insert("mpris:artUrl", url.to_variant());
        }
    } else {
        metadata.insert(
            "mpris:trackid",
            ObjectPath::try_from(NO_TRACK).unwrap().to_variant(),
        );
    }
    let status = if player.playing.get() {
        "Playing"
    } else if player.active.get() {
        "Paused"
    } else {
        "Stopped"
    };
    let repeat = match state.repeat {
        Repeat::Off => "None",
        Repeat::Track => "Track",
        Repeat::Queue => "Playlist",
    };
    BTreeMap::from([
        ("PlaybackStatus", status.to_variant()),
        ("LoopStatus", repeat.to_variant()),
        ("Rate", 1.0_f64.to_variant()),
        ("MinimumRate", 1.0_f64.to_variant()),
        ("MaximumRate", 1.0_f64.to_variant()),
        ("Shuffle", state.shuffle.to_variant()),
        ("Metadata", metadata.to_variant()),
        ("Volume", (player.volume.value() / 100.0).to_variant()),
        (
            "CanGoNext",
            state.next_index(1, false).is_some().to_variant(),
        ),
        (
            "CanGoPrevious",
            state.next_index(-1, false).is_some().to_variant(),
        ),
        ("CanPlay", track.is_some().to_variant()),
        ("CanPause", player.active.get().to_variant()),
        (
            "CanSeek",
            (player.active.get() && duration > 0).to_variant(),
        ),
        ("CanControl", true.to_variant()),
    ])
}

fn track_id(index: usize, track: &std::path::Path) -> ObjectPath {
    let mut hash = DefaultHasher::new();
    (index, track).hash(&mut hash);
    ObjectPath::try_from(format!(
        "/org/mpris/MediaPlayer2/track/t{:016x}",
        hash.finish()
    ))
    .unwrap()
}

fn duration(player: &Player) -> i64 {
    player
        .pipeline
        .query_duration::<gst::ClockTime>()
        .and_then(|time| i64::try_from(time.useconds()).ok())
        .unwrap_or(0)
}

fn position(player: &Player) -> i64 {
    if !player.active.get() {
        return 0;
    }
    player
        .pipeline
        .query_position::<gst::ClockTime>()
        .and_then(|time| i64::try_from(time.useconds()).ok())
        .unwrap_or(0)
}

fn player_command(player: &Rc<Player>, method: &str, args: &glib::Variant) {
    match method {
        "Next" => player.step(1),
        "Previous" => player.step(-1),
        "Play" if !player.playing.get() => player.toggle(),
        "Pause" if player.playing.get() => player.toggle(),
        "PlayPause" => player.toggle(),
        "Stop" => player.stop(),
        "Seek" if player.active.get() => {
            if let Some((offset,)) = args.get::<(i64,)>() {
                let length = duration(player);
                if length > 0 {
                    let target = position(player).saturating_add(offset).max(0);
                    if target > length {
                        player.step(1);
                    } else {
                        player.seek(target as f64 / 1_000_000.0);
                    }
                }
            }
        }
        "SetPosition" if player.active.get() => {
            if let Some((id, target)) = args.get::<(ObjectPath, i64)>() {
                let length = duration(player);
                let valid_id = {
                    let state = player.state.borrow();
                    state
                        .queue
                        .get(state.current)
                        .is_some_and(|path| track_id(state.current, path) == id)
                };
                if valid_id && length > 0 && (0..=length).contains(&target) {
                    player.seek(target as f64 / 1_000_000.0);
                }
            }
        }
        "OpenUri" => {
            if let Some((uri,)) = args.get::<(String,)>() {
                if uri.starts_with("file://") {
                    if let Some(path) = gio::File::for_uri(&uri)
                        .path()
                        .filter(|path| path.is_file() && supported(path))
                    {
                        player.import(vec![path], Import::Play);
                    }
                }
            }
        }
        _ => {}
    }
    player.update_transport();
}

fn set_property(player: &Rc<Player>, property: &str, value: glib::Variant) -> bool {
    match property {
        "Shuffle" => {
            let Some(enabled) = value.get::<bool>() else {
                return false;
            };
            player.shuffle.set_active(enabled);
        }
        "Volume" => {
            let Some(volume) = value.get::<f64>().filter(|value| value.is_finite()) else {
                return false;
            };
            player.volume.set_value(volume.clamp(0.0, 1.0) * 100.0);
        }
        "LoopStatus" => {
            let repeat = match value.str() {
                Some("None") => Repeat::Off,
                Some("Track") => Repeat::Track,
                Some("Playlist") => Repeat::Queue,
                _ => return false,
            };
            player.state.borrow_mut().repeat = repeat;
            player.update_repeat();
            player.save();
        }
        "Rate" => return value.get::<f64>() == Some(1.0),
        _ => return false,
    }
    player.update_transport();
    true
}
