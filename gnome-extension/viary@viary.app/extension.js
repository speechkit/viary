// Viary's GNOME Shell extension. Wayland lets no app type into another or
// float a window above the rest; the shell can do both. Viary calls this
// over D-Bus to paste (Ctrl+V), undo (Ctrl+Z), learn which app has focus,
// and show its dictation pill, whose buttons come back as a signal.
//
// Only Viary may call it: the client that owns VIARY_NAME on the session
// bus. Other programs get AccessDenied, so apps on the bus cannot use the
// shell's keyboard and focus, which Wayland keeps from them. This is not
// a defense against a hostile program running as the user: one could own
// the name before Viary does, but it could as well rewrite this file,
// which lives in the user's home.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const BUS_NAME = 'app.viary.Shell';
const OBJECT_PATH = '/app/viary/Shell';
/** The name Viary's own connection owns: its calls are the ones answered. */
const VIARY_NAME = 'app.viary.App';
/** Bumped when the interface changes, so Viary can ask for an update. */
const VERSION = 2;

/** Keys that would turn Ctrl+V into another shortcut while held: the talk
 *  shortcut's Alt can still be down when its Space comes up. Ctrl is not
 *  here: Ctrl+V is still Ctrl+V. */
const IN_THE_WAY = Clutter.ModifierType.SHIFT_MASK | Clutter.ModifierType.MOD1_MASK |
    Clutter.ModifierType.MOD4_MASK | Clutter.ModifierType.MOD5_MASK |
    Clutter.ModifierType.SUPER_MASK | Clutter.ModifierType.META_MASK;
/** How long to wait for those keys to come up, in ms. */
const WAIT_FOR_KEYS = 1500;

const INTERFACE = `
<node>
  <interface name="app.viary.Shell">
    <method name="Paste"/>
    <method name="Undo"/>
    <method name="FocusedApp">
      <arg type="i" direction="out" name="pid"/>
      <arg type="s" direction="out" name="name"/>
    </method>
    <method name="ShowPill">
      <arg type="s" direction="in" name="view"/>
    </method>
    <method name="SetLevel">
      <arg type="d" direction="in" name="level"/>
    </method>
    <method name="SetPartial">
      <arg type="t" direction="in" name="token"/>
      <arg type="s" direction="in" name="text"/>
    </method>
    <signal name="PillAction">
      <arg type="s" name="action"/>
    </signal>
    <property name="Version" type="u" access="read"/>
  </interface>
</node>`;

const BARS = 22;

/** Microphone RMS to a bar height in px: -60 dB is silence, -15 dB is loud. */
function barHeight(level) {
    const db = 20 * Math.log10(Math.max(level, 1e-5));
    const norm = Math.min(1, Math.max(0, (db + 60) / 45));
    return 4 + Math.round(norm * 24);
}

/** The tail of the live text: older words muted, the newest two bright. */
function tail(text) {
    const trimmed = text.trim();
    if (!trimmed)
        return ['', ''];
    if (/\s/.test(trimmed)) {
        const words = trimmed.split(/\s+/).slice(-9);
        return [words.slice(0, -2).join(' '), words.slice(-2).join(' ')];
    }
    // Chinese and Japanese have no spaces: the last characters.
    const chars = Array.from(trimmed).slice(-22);
    return [chars.slice(0, -4).join(''), chars.slice(-4).join('')];
}

function clock(ms) {
    const s = Math.max(0, Math.floor(ms / 1000));
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

/** The pill: bottom center of the primary monitor, above every window. */
class Pill {
    constructor(onAction) {
        this._onAction = onAction;
        this._actor = new St.BoxLayout({
            style_class: 'viary-pill',
            reactive: true,
            visible: false,
            y_align: Clutter.ActorAlign.CENTER,
        });
        // Above every window, taking clicks. GNOME 50 rejects the
        // affectsInputRegion option that earlier versions defaulted to on.
        Main.layoutManager.addTopChrome(this._actor);
        this._actor.connect('notify::width', () => this._place());
        this._levels = [];
        this._view = {kind: 'idle'};
        this._text = '';
    }

    destroy() {
        this._stopClock();
        Main.layoutManager.removeChrome(this._actor);
        this._actor.destroy();
    }

    _place() {
        const monitor = Main.layoutManager.primaryMonitor;
        if (!monitor)
            return;
        const [width, height] = [this._actor.width, this._actor.height];
        this._actor.set_position(
            Math.round(monitor.x + (monitor.width - width) / 2),
            Math.round(monitor.y + monitor.height - height - 64));
    }

    _label(text, styleClass) {
        return new St.Label({text, style_class: styleClass ?? '', y_align: Clutter.ActorAlign.CENTER});
    }

    _chip(text, action, primary = false) {
        const button = new St.Button({
            label: text,
            style_class: primary ? 'viary-chip viary-chip-primary' : 'viary-chip',
            y_align: Clutter.ActorAlign.CENTER,
            can_focus: false,
        });
        button.connect('clicked', () => this._onAction(action));
        return button;
    }

    show(view) {
        this._view = view;
        this._stopClock();
        this._actor.destroy_all_children();
        this._bars = null;
        this._older = null;
        this._newest = null;
        if (view.kind === 'idle') {
            this._actor.hide();
            return;
        }
        const listening = view.kind === 'listening' || view.kind === 'handsFree';
        if (listening) {
            if (view.token !== this._token) {
                this._token = view.token;
                this._text = '';
                this._levels = [];
            }
            this._actor.add_child(new St.Widget({style_class: 'viary-dot', y_align: Clutter.ActorAlign.CENTER}));
            this._bars = new St.BoxLayout({style_class: 'viary-bars', y_align: Clutter.ActorAlign.CENTER});
            for (let i = 0; i < BARS; i++)
                this._bars.add_child(new St.Widget({style_class: 'viary-bar', y_align: Clutter.ActorAlign.CENTER}));
            this._actor.add_child(this._bars);
            this._drawLevels();
            if (view.live) {
                const text = new St.BoxLayout({y_align: Clutter.ActorAlign.CENTER});
                this._older = this._label('', 'viary-older');
                this._newest = this._label('');
                text.add_child(this._older);
                text.add_child(this._newest);
                this._actor.add_child(text);
                this._drawText();
            }
            const time = this._label(clock(Date.now() - view.startedAt), 'viary-clock');
            this._actor.add_child(time);
            this._clock = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 1, () => {
                time.text = clock(Date.now() - view.startedAt);
                return GLib.SOURCE_CONTINUE;
            });
            if (view.context)
                this._actor.add_child(this._label(view.context, 'viary-context'));
            if (view.kind === 'handsFree')
                this._actor.add_child(this._chip('Stop', 'stop', true));
        } else if (view.kind === 'transcribing') {
            this._actor.add_child(this._label(view.label));
        } else if (view.kind === 'polishing') {
            this._actor.add_child(this._label('Polishing'));
            this._actor.add_child(this._chip('Skip', 'skipPolish'));
        } else if (view.kind === 'inserted') {
            this._actor.add_child(this._label(view.label));
            this._actor.add_child(this._chip('Undo', 'undo'));
            if (view.canRaw)
                this._actor.add_child(this._chip('Use raw', 'useRaw'));
        } else if (view.kind === 'copied') {
            this._actor.add_child(this._label(view.label));
            this._actor.add_child(this._label(view.hint, 'viary-clock'));
        } else if (view.kind === 'failed') {
            this._actor.add_child(this._label(view.message));
            if (view.retryable)
                this._actor.add_child(this._chip('Retry', 'retry', true));
            if (view.alternative)
                this._actor.add_child(this._chip(view.alternative, 'switchEngine'));
            this._actor.add_child(this._chip('Discard', 'dismiss'));
        } else if (view.kind === 'hint') {
            this._actor.add_child(this._label(view.text));
        }
        this._actor.show();
        this._place();
    }

    level(level) {
        this._levels.push(level);
        if (this._levels.length > BARS)
            this._levels.shift();
        this._drawLevels();
    }

    partial(token, text) {
        if (token !== this._token)
            return;
        this._text = text;
        this._drawText();
    }

    _drawLevels() {
        if (!this._bars)
            return;
        const bars = this._bars.get_children();
        const offset = BARS - this._levels.length;
        bars.forEach((bar, i) => {
            bar.height = i < offset ? 4 : barHeight(this._levels[i - offset]);
        });
    }

    _drawText() {
        if (!this._older)
            return;
        const [older, newest] = tail(this._text);
        this._older.text = older ? `${older} ` : '';
        this._newest.text = newest || 'Listening…';
    }

    _stopClock() {
        if (this._clock) {
            GLib.source_remove(this._clock);
            this._clock = 0;
        }
    }
}

export default class ViaryExtension extends Extension {
    enable() {
        const seat = Clutter.get_default_backend().get_default_seat();
        this._keyboard = seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
        this._pill = new Pill(action => {
            this._dbus?.emit_signal('PillAction', new GLib.Variant('(s)', [action]));
        });
        this._dbus = Gio.DBusExportedObject.wrapJSObject(INTERFACE, this);
        this._dbus.export(Gio.DBus.session, OBJECT_PATH);
        this._name = Gio.bus_own_name(Gio.BusType.SESSION, BUS_NAME, Gio.BusNameOwnerFlags.NONE, null, null, null);
        this._viary = null;
        this._watch = Gio.bus_watch_name(Gio.BusType.SESSION, VIARY_NAME, Gio.BusNameWatcherFlags.NONE,
            (_connection, _name, owner) => {
                this._viary = owner;
            },
            () => {
                this._viary = null;
            });
    }

    disable() {
        if (this._watch) {
            Gio.bus_unwatch_name(this._watch);
            this._watch = 0;
        }
        this._viary = null;
        if (this._name) {
            Gio.bus_unown_name(this._name);
            this._name = 0;
        }
        this._dbus?.unexport();
        this._dbus = null;
        this._pill?.destroy();
        this._pill = null;
        this._keyboard?.run_dispose();
        this._keyboard = null;
    }

    get Version() {
        return VERSION;
    }

    /** Runs `then` if `invocation` comes from Viary; answers AccessDenied
     *  if not. A sender other than the one the name watch last saw is
     *  asked about first: Viary claims its name just before its first call,
     *  which can come before the watch hears of it. */
    _fromViary(invocation, then) {
        const sender = invocation.get_sender();
        if (this._viary && sender === this._viary) {
            then();
            return;
        }
        Gio.DBus.session.call('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
            'GetNameOwner', new GLib.Variant('(s)', [VIARY_NAME]), new GLib.VariantType('(s)'),
            Gio.DBusCallFlags.NONE, -1, null, (connection, result) => {
                let owner = null;
                try {
                    [owner] = connection.call_finish(result).deepUnpack();
                } catch {
                    // No owner: Viary is not running.
                }
                if (owner && owner === sender && this._pill) {
                    this._viary = owner;
                    then();
                } else {
                    invocation.return_dbus_error('org.freedesktop.DBus.Error.AccessDenied',
                        `Only the client that owns ${VIARY_NAME} may call this`);
                }
            });
    }

    /** Presses `keyvals` in order, then releases them in reverse. */
    _press(keyvals) {
        const time = GLib.get_monotonic_time();
        for (const keyval of keyvals)
            this._keyboard.notify_keyval(time, keyval, Clutter.KeyState.PRESSED);
        for (const keyval of [...keyvals].reverse())
            this._keyboard.notify_keyval(time, keyval, Clutter.KeyState.RELEASED);
    }

    /** Presses `keyvals` once none of IN_THE_WAY is held, then answers;
     *  an error if one still is after WAIT_FOR_KEYS. */
    _pressWhenReleased(keyvals, invocation) {
        const deadline = GLib.get_monotonic_time() + WAIT_FOR_KEYS * 1000;
        const attempt = () => {
            if (!this._keyboard) {
                invocation.return_dbus_error('app.viary.Shell.Error.Disabled', 'The extension was turned off');
                return GLib.SOURCE_REMOVE;
            }
            const [, , mods] = global.get_pointer();
            if (mods & IN_THE_WAY) {
                if (GLib.get_monotonic_time() < deadline)
                    return GLib.SOURCE_CONTINUE;
                invocation.return_dbus_error('app.viary.Shell.Error.KeyHeld', 'A modifier key is still held');
                return GLib.SOURCE_REMOVE;
            }
            this._press(keyvals);
            invocation.return_value(null);
            return GLib.SOURCE_REMOVE;
        };
        if (attempt() === GLib.SOURCE_CONTINUE)
            GLib.timeout_add(GLib.PRIORITY_DEFAULT, 15, attempt);
    }

    // Each method is the `Async` form, which gets the invocation and so
    // its sender.

    PasteAsync(_params, invocation) {
        this._fromViary(invocation, () =>
            this._pressWhenReleased([Clutter.KEY_Control_L, Clutter.KEY_v], invocation));
    }

    UndoAsync(_params, invocation) {
        this._fromViary(invocation, () =>
            this._pressWhenReleased([Clutter.KEY_Control_L, Clutter.KEY_z], invocation));
    }

    FocusedAppAsync(_params, invocation) {
        this._fromViary(invocation, () => {
            const window = global.display.focus_window;
            const app = window ? Shell.WindowTracker.get_default().get_window_app(window) : null;
            const focused = window
                ? [window.get_pid(), app?.get_name() ?? window.get_wm_class() ?? '']
                : [0, ''];
            invocation.return_value(new GLib.Variant('(is)', focused));
        });
    }

    ShowPillAsync([view], invocation) {
        this._fromViary(invocation, () => {
            try {
                this._pill?.show(JSON.parse(view));
            } catch (error) {
                logError(error, 'Viary: cannot show the pill');
            }
            invocation.return_value(null);
        });
    }

    SetLevelAsync([level], invocation) {
        this._fromViary(invocation, () => {
            this._pill?.level(level);
            invocation.return_value(null);
        });
    }

    SetPartialAsync([token, text], invocation) {
        this._fromViary(invocation, () => {
            this._pill?.partial(Number(token), text);
            invocation.return_value(null);
        });
    }
}
