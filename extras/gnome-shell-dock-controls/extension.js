// SPDX-License-Identifier: GPL-3.0-only
// Shows Ferritone's transport controls when the pointer rests on its dock icon.
// Works with Dash to Dock / Ubuntu Dock and with the GNOME overview dash.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Pango from 'gi://Pango';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const APP_ID = 'io.github.ghsantosalves.Ferritone.desktop';
const BUS_NAME = 'org.mpris.MediaPlayer2.Ferritone';
const OBJECT_PATH = '/org/mpris/MediaPlayer2';
const PLAYER_IFACE = 'org.mpris.MediaPlayer2.Player';
const SHOW_DELAY_MS = 300;
const HIDE_DELAY_MS = 300;
const GAP = 8;
const MAX_ANCESTORS = 8;

const STRINGS = {
    en: {previous: 'Previous track', play: 'Play', pause: 'Pause', next: 'Next track'},
    pt: {previous: 'Faixa anterior', play: 'Tocar', pause: 'Pausar', next: 'Próxima faixa'},
};
const tr = STRINGS[GLib.get_language_names()[0].startsWith('pt') ? 'pt' : 'en'];

function isFerritoneDockIcon(actor) {
    if (actor?.app?.get_id?.() !== APP_ID)
        return false;
    // Dock icons (DockAppIcon) and overview dash icons (DashIcon), not the app grid.
    return /Dock|Dash/.test(GObject.type_name(actor.constructor.$gtype));
}

export default class FerritoneDockControls extends Extension {
    enable() {
        this._icons = new Map();
        this._activeIcon = null;
        this._suppressedLabel = null;
        this._lastEventActor = null;
        this._showId = 0;
        this._hideId = 0;
        this._proxy = null;
        this._proxySignals = [];
        this._cancellable = new Gio.Cancellable();

        this._buildPopup();
        this._connectPlayer();
        this._stageId = global.stage.connect('captured-event',
            (_stage, event) => this._onCapturedEvent(event));
    }

    disable() {
        global.stage.disconnect(this._stageId);
        this._cancellable.cancel();
        this._clearTimeouts();
        this._restoreLabel();
        for (const [icon, ids] of this._icons)
            ids.forEach(id => icon.disconnect(id));
        this._icons.clear();
        if (this._proxy)
            this._proxySignals.forEach(id => this._proxy.disconnect(id));
        Main.layoutManager.removeChrome(this._popup);
        this._popup.destroy();
        this._popup = this._proxy = this._activeIcon = this._lastEventActor = null;
        this._proxySignals = [];
    }

    _buildPopup() {
        this._popup = new St.BoxLayout({
            style_class: 'ferritone-dock-controls',
            orientation: Clutter.Orientation.VERTICAL,
            reactive: true,
            track_hover: true,
            visible: false,
        });
        this._title = new St.Label({style_class: 'ferritone-dock-controls-title', x_align: Clutter.ActorAlign.CENTER});
        this._artist = new St.Label({style_class: 'ferritone-dock-controls-artist', x_align: Clutter.ActorAlign.CENTER});
        for (const label of [this._title, this._artist])
            label.clutter_text.ellipsize = Pango.EllipsizeMode.END;

        const row = new St.BoxLayout({style_class: 'ferritone-dock-controls-buttons', x_align: Clutter.ActorAlign.CENTER});
        this._previous = this._button('media-skip-backward-symbolic', tr.previous, 'Previous');
        this._playPause = this._button('media-playback-start-symbolic', tr.play, 'PlayPause');
        this._playPause.add_style_class_name('ferritone-dock-controls-play');
        this._next = this._button('media-skip-forward-symbolic', tr.next, 'Next');
        [this._previous, this._playPause, this._next].forEach(button => row.add_child(button));

        this._popup.add_child(this._title);
        this._popup.add_child(this._artist);
        this._popup.add_child(row);
        this._popup.connect('notify::hover', () => this._onHoverChanged());
        Main.layoutManager.addTopChrome(this._popup);
    }

    _button(iconName, accessibleName, method) {
        const button = new St.Button({
            style_class: 'ferritone-dock-controls-button',
            can_focus: true,
            accessible_name: accessibleName,
            child: new St.Icon({icon_name: iconName, icon_size: 16}),
        });
        button.connect('clicked', () => this._call(method));
        return button;
    }

    _connectPlayer() {
        Gio.DBusProxy.new_for_bus(
            Gio.BusType.SESSION, Gio.DBusProxyFlags.DO_NOT_AUTO_START, null,
            BUS_NAME, OBJECT_PATH, PLAYER_IFACE, this._cancellable,
            (_source, result) => {
                try {
                    this._proxy = Gio.DBusProxy.new_for_bus_finish(result);
                } catch (e) {
                    if (!e.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED))
                        console.error(`Ferritone dock controls: ${e.message}`);
                    return;
                }
                this._proxySignals = [
                    this._proxy.connect('g-properties-changed', () => this._sync()),
                    this._proxy.connect('notify::g-name-owner', () => {
                        if (this._proxy.g_name_owner)
                            this._sync();
                        else
                            this._hide();
                    }),
                ];
            });
    }

    _call(method) {
        this._proxy?.call(method, null, Gio.DBusCallFlags.NO_AUTO_START, 2000, null, (proxy, result) => {
            try {
                proxy.call_finish(result);
            } catch (e) {
                console.error(`Ferritone dock controls: ${method} failed: ${e.message}`);
            }
        });
    }

    _onCapturedEvent(event) {
        const type = event.type();
        if (type !== Clutter.EventType.ENTER && type !== Clutter.EventType.MOTION)
            return Clutter.EVENT_PROPAGATE;
        const actor = global.stage.get_event_actor?.(event) ?? event.get_source?.();
        if (!actor || actor === this._lastEventActor)
            return Clutter.EVENT_PROPAGATE;
        this._lastEventActor = actor;
        let candidate = actor;
        for (let depth = 0; candidate && depth < MAX_ANCESTORS; depth++, candidate = candidate.get_parent()) {
            if (isFerritoneDockIcon(candidate)) {
                if (!this._icons.has(candidate))
                    this._track(candidate);
                break;
            }
        }
        return Clutter.EVENT_PROPAGATE;
    }

    _track(icon) {
        const ids = [
            icon.connect('notify::hover', () => this._onHoverChanged()),
            icon.connect('destroy', () => {
                this._icons.delete(icon);
                if (this._activeIcon === icon)
                    this._hide();
            }),
        ];
        if (GObject.signal_lookup('menu-state-changed', icon.constructor.$gtype))
            ids.push(icon.connect('menu-state-changed', (_icon, opened) => opened && this._hide()));
        this._icons.set(icon, ids);
        this._onHoverChanged();
    }

    _hoveredIcon() {
        for (const icon of this._icons.keys()) {
            if (icon.hover && icon.mapped)
                return icon;
        }
        return null;
    }

    _onHoverChanged() {
        const icon = this._hoveredIcon();
        if (icon || this._popup.hover) {
            this._cancelHide();
            if (icon && icon !== this._activeIcon)
                this._scheduleShow(icon);
        } else {
            this._cancelShow();
            this._scheduleHide();
        }
    }

    _scheduleShow(icon) {
        this._cancelShow();
        this._showId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, SHOW_DELAY_MS, () => {
            this._showId = 0;
            if (icon.hover && this._proxy?.g_name_owner)
                this._show(icon);
            return GLib.SOURCE_REMOVE;
        });
    }

    _scheduleHide() {
        if (this._hideId || !this._activeIcon)
            return;
        this._hideId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, HIDE_DELAY_MS, () => {
            this._hideId = 0;
            if (!this._hoveredIcon() && !this._popup.hover)
                this._hide();
            return GLib.SOURCE_REMOVE;
        });
    }

    _cancelShow() {
        if (this._showId)
            GLib.source_remove(this._showId);
        this._showId = 0;
    }

    _cancelHide() {
        if (this._hideId)
            GLib.source_remove(this._hideId);
        this._hideId = 0;
    }

    _clearTimeouts() {
        this._cancelShow();
        this._cancelHide();
    }

    _show(icon) {
        if (icon.get_parent()?._menu?.isOpen || icon._menu?.isOpen)
            return;
        this._restoreLabel();
        this._activeIcon = icon;
        this._sync();
        this._suppressLabel(icon);
        this._popup.remove_all_transitions();
        this._popup.opacity = 0;
        this._popup.show();
        this._place(icon);
        this._popup.ease({opacity: 255, duration: 120, mode: Clutter.AnimationMode.EASE_OUT_QUAD});
    }

    _hide() {
        this._clearTimeouts();
        this._restoreLabel();
        this._activeIcon = null;
        if (!this._popup?.visible)
            return;
        this._popup.remove_all_transitions();
        this._popup.ease({
            opacity: 0,
            duration: 100,
            mode: Clutter.AnimationMode.EASE_OUT_QUAD,
            onComplete: () => this._popup?.hide(),
        });
    }

    // The dock shows the app name above the icon on hover; keep it from overlapping the controls.
    _suppressLabel(icon) {
        const container = icon.get_parent();
        if (typeof container?.showLabel !== 'function' || typeof container.hideLabel !== 'function')
            return;
        container.showLabel = () => {};
        container.hideLabel();
        this._suppressedLabel = container;
    }

    _restoreLabel() {
        if (this._suppressedLabel)
            delete this._suppressedLabel.showLabel;
        this._suppressedLabel = null;
    }

    _place(icon) {
        const [, width] = this._popup.get_preferred_width(-1);
        const [, height] = this._popup.get_preferred_height(width);
        const [x, y] = icon.get_transformed_position();
        const [w, h] = icon.get_transformed_size();
        const monitor = Main.layoutManager.findMonitorForActor(icon) ?? Main.layoutManager.primaryMonitor;
        const distances = {
            bottom: monitor.y + monitor.height - (y + h),
            top: y - monitor.y,
            left: x - monitor.x,
            right: monitor.x + monitor.width - (x + w),
        };
        const edge = Object.keys(distances).reduce((a, b) => (distances[a] <= distances[b] ? a : b));
        let px, py;
        switch (edge) {
        case 'top':
            [px, py] = [x + w / 2 - width / 2, y + h + GAP];
            break;
        case 'left':
            [px, py] = [x + w + GAP, y + h / 2 - height / 2];
            break;
        case 'right':
            [px, py] = [x - width - GAP, y + h / 2 - height / 2];
            break;
        default:
            [px, py] = [x + w / 2 - width / 2, y - height - GAP];
        }
        px = Math.min(Math.max(px, monitor.x + GAP), monitor.x + monitor.width - width - GAP);
        py = Math.min(Math.max(py, monitor.y + GAP), monitor.y + monitor.height - height - GAP);
        this._popup.set_position(Math.round(px), Math.round(py));
    }

    _sync() {
        if (!this._proxy?.g_name_owner) {
            this._hide();
            return;
        }
        const value = name => this._proxy.get_cached_property(name)?.deepUnpack();
        const metadata = value('Metadata') ?? {};
        const field = key => metadata[key]?.deepUnpack();
        const artists = field('xesam:artist');
        const artist = Array.isArray(artists) ? artists.join(', ') : '';

        this._title.text = field('xesam:title') || 'Ferritone';
        this._artist.text = artist;
        this._artist.visible = artist.length > 0;

        const playing = value('PlaybackStatus') === 'Playing';
        this._playPause.child.icon_name = playing ? 'media-playback-pause-symbolic' : 'media-playback-start-symbolic';
        this._playPause.accessible_name = playing ? tr.pause : tr.play;
        this._playPause.reactive = value('CanPlay') !== false || playing;
        this._previous.reactive = value('CanGoPrevious') !== false;
        this._next.reactive = value('CanGoNext') !== false;

        if (this._popup.visible && this._activeIcon)
            this._place(this._activeIcon);
    }
}
