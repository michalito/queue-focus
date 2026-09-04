// Queue Focus — top-bar indicator talking to the queue-focus service over D-Bus.
import GObject from 'gi://GObject';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';
import Clutter from 'gi://Clutter';
import Meta from 'gi://Meta';
import Pango from 'gi://Pango';
import Shell from 'gi://Shell';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import {FlashOverlay} from './flash.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as MessageTray from 'resource:///org/gnome/shell/ui/messageTray.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

const APP_NAME = 'Queue Focus';
const APP_ICON = 'org.queuefocus.QueueFocus-symbolic';
const BUS_NAME = 'org.queuefocus.QueueFocus';
const OBJ_PATH = '/org/queuefocus/QueueFocus';
// The part of org.queuefocus.QueueFocus1 this indicator uses.
const IFACE_XML = `
<node>
  <interface name="org.queuefocus.QueueFocus1">
    <method name="GetState"><arg type="s" name="json" direction="out"/></method>
    <method name="Add"><arg type="s" name="text" direction="in"/><arg type="s" name="bucket" direction="in"/><arg type="t" name="id" direction="out"/></method>
    <method name="CompleteCurrent"><arg type="t" name="id" direction="out"/><arg type="s" name="title" direction="out"/></method>
    <method name="UndoComplete"><arg type="t" name="id" direction="in"/><arg type="b" name="undone" direction="out"/></method>
    <method name="Promote"><arg type="t" name="id" direction="in"/></method>
    <method name="Show"><arg type="s" name="view" direction="in"/></method>
    <method name="GetSettings"><arg type="s" name="json" direction="out"/></method>
    <signal name="Changed"><arg type="s" name="json"/></signal>
    <signal name="SettingsChanged"><arg type="s" name="json"/></signal>
    <signal name="Flash"><arg type="s" name="json"/></signal>
    <signal name="DurabilityWarning"><arg type="s" name="message"/></signal>
    <signal name="Stopping"/>
  </interface>
</node>`;
const QueueFocusProxy = Gio.DBusProxy.makeProxyWrapper(IFACE_XML);

// Delay before asking the service for state again after it went away without
// notice or a call failed. Doubles per attempt up to the max so a binary that
// crashes at startup is not respawned in a tight loop; reset once state flows.
const RETRY_MS = 1500;
const RETRY_MAX_MS = 60000;
// How many Next tasks the menu lists before summarising the rest.
const NEXT_PREVIEW = 8;
// gschema key → what to do when pressed.
const KEYBINDINGS = {
    'toggle-queue': ind => ind.call('Show', 'toggle'),
    'quick-add': ind => ind.call('Show', 'add'),
    'show-board': ind => ind.call('Show', 'board'),
    'complete-current': ind => ind.completeCurrent(),
};

/** "12m" or "1h02"; a paused task keeps the time it had and shows ⏸. */
function elapsed(startedAt, pausedAt) {
    const s = Math.max(0, (pausedAt || Math.floor(Date.now() / 1000)) - startedAt);
    const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60);
    const t = h > 0 ? `${h}h${String(m).padStart(2, '0')}` : `${m}m`;
    return pausedAt ? `${t} ⏸` : t;
}

const Indicator = GObject.registerClass(
class QueueFocusIndicator extends PanelMenu.Button {
    _init() {
        super._init(0.5, APP_NAME);
        this._state = null;
        // What the user chose on the app's Settings page. Until the service
        // answers, the top bar shows the clock, as it always has.
        this._prefs = {show_timer: true};
        this._flash = new FlashOverlay();
        this._quickAddPending = false;
        this._entry = null;
        // Focus key → actor, for the menu currently built (see _buildMenu).
        this._focusTargets = new Map();
        this._focusKey = null;
        this._focusId = 0;
        this._tickId = 0;
        this._retryId = 0;
        this._retryMs = RETRY_MS;
        // Set when the service says it is exiting on request, cleared when it is back.
        this._stopping = false;
        this._source = null;
        // The notification offering to undo the latest completion, if still shown.
        this._doneNotification = null;

        const box = new St.BoxLayout({style_class: 'qf-box'});
        this._dot = new St.Label({text: '●', style_class: 'qf-dot', y_align: Clutter.ActorAlign.CENTER});
        this._label = new St.Label({text: '…', style_class: 'qf-label', y_align: Clutter.ActorAlign.CENTER});
        this._label.clutter_text.ellipsize = Pango.EllipsizeMode.END;
        this._timer = new St.Label({text: '', style_class: 'qf-timer', y_align: Clutter.ActorAlign.CENTER});
        box.add_child(this._dot);
        box.add_child(this._label);
        box.add_child(this._timer);
        this.add_child(box);

        this._proxy = new QueueFocusProxy(Gio.DBus.session, BUS_NAME, OBJ_PATH, (_proxy, error) => {
            if (error) {
                console.warn(`queue-focus: proxy error: ${error.message}`);
                this._apply(null);
                return;
            }
            this._refresh();
        });
        this._signalIds = [
            this._proxy.connectSignal('Changed', (_p, _s, [json]) => this._apply(json)),
            this._proxy.connectSignal('SettingsChanged', (_p, _s, [json]) => this._applySettings(json)),
            this._proxy.connectSignal('Flash', (_p, _s, [json]) => this._onFlash(json)),
            this._proxy.connectSignal('DurabilityWarning',
                (_p, _s, [message]) => Main.notifyError(APP_NAME, message)),
            this._proxy.connectSignal('Stopping', () => {
                this._stopping = true;
            }),
        ];
        this._ownerId = this._proxy.connect('notify::g-name-owner', () => this._onOwnerChanged());

        this.menu.connect('open-state-changed', (_m, open) => {
            if (open && !this._quickAddPending) this._buildMenu(true);
        });
    }

    // ---- service lifecycle ------------------------------------------------

    _onOwnerChanged() {
        if (this._proxy.g_name_owner) {
            this._stopping = false;
            this._cancelRetry();
            this._refresh();
            return;
        }
        this._apply(null);
        // Gone without notice (crashed or killed): bring it back. A service
        // that announced it was stopping stays down until the next request.
        if (!this._stopping) this._scheduleRetry();
    }

    /** Ask for state. Like any method call, this starts the service via D-Bus activation. */
    _refresh() {
        this._proxy?.GetSettingsRemote((res, err) => {
            if (!this._proxy || err) return;
            this._applySettings(res[0]);
        });
        this._proxy?.GetStateRemote((res, err) => {
            if (!this._proxy) return;
            if (!err) {
                this._apply(res[0]);
                return;
            }
            this._apply(null);
            if (err.matches(Gio.DBusError, Gio.DBusError.SERVICE_UNKNOWN)) {
                // Nothing to activate (not installed, or uninstalled under us):
                // stay idle until the user asks again.
                console.warn(`queue-focus: service unavailable: ${err.message}`);
                return;
            }
            console.warn(`queue-focus: GetState failed: ${err.message}`);
            this._scheduleRetry();
        });
    }

    _scheduleRetry() {
        if (!this._proxy || this._retryId) return;
        this._retryId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, this._retryMs, () => {
            this._retryId = 0;
            this._refresh();
            return GLib.SOURCE_REMOVE;
        });
        this._retryMs = Math.min(this._retryMs * 2, RETRY_MAX_MS);
    }

    _cancelRetry() {
        if (!this._retryId) return;
        GLib.source_remove(this._retryId);
        this._retryId = 0;
    }

    // ---- state → panel ----------------------------------------------------

    _apply(json) {
        try {
            this._state = json ? JSON.parse(json) : null;
        } catch (_e) {
            this._state = null;
        }
        if (this._state) {
            this._retryMs = RETRY_MS;
            this._cancelRetry();
        }
        const cur = this._state?.current ?? null;
        for (const c of ['qf-dot-work', 'qf-dot-personal', 'qf-dot-none']) this._dot.remove_style_class_name(c);
        this._dot.add_style_class_name(cur?.tag ? `qf-dot-${cur.tag}` : 'qf-dot-none');
        this._label.text = cur ? cur.title : (this._state ? 'no task' : 'queue-focus');
        this._updateTimer();
        if (this.menu.isOpen && !this._quickAddPending) this._buildMenu();
    }

    _applySettings(json) {
        try {
            const settings = JSON.parse(json);
            if (settings && typeof settings === 'object') this._prefs = settings;
        } catch (e) {
            console.warn(`queue-focus: unreadable settings: ${e.message}`);
            return;
        }
        this._updateTimer();
    }

    /** The service says it is time to remind the user what they are doing. */
    _onFlash(json) {
        let event;
        try {
            event = JSON.parse(json);
        } catch (e) {
            console.warn(`queue-focus: unreadable flash: ${e.message}`);
            return;
        }
        if (event?.title) this._flash.show(event);
    }

    /** Show the clock, then wake up right after its next minute boundary. */
    _updateTimer() {
        this._cancelTick();
        const cur = this._state?.current;
        const wanted = this._prefs.show_timer !== false;
        const showing = wanted && !!cur?.started_at;
        this._timer.text = showing ? elapsed(cur.started_at, cur.paused_at) : '';
        // An empty label still carries its margin, so take it out of the box.
        this._timer.visible = showing;
        if (!showing || cur.paused_at) return;
        const secs = Math.max(0, Math.floor(Date.now() / 1000) - cur.started_at);
        this._tickId = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 60 - (secs % 60), () => {
            this._tickId = 0;
            this._updateTimer();
            return GLib.SOURCE_REMOVE;
        });
    }

    _cancelTick() {
        if (!this._tickId) return;
        GLib.source_remove(this._tickId);
        this._tickId = 0;
    }

    // ---- actions ----------------------------------------------------------

    /** Fire-and-forget method call; a failure is shown to the user, not just logged. */
    call(name, ...args) {
        this._proxy?.[`${name}Remote`](...args, (_res, err) => {
            if (err) this._fail(name, err);
        });
    }

    _fail(name, err) {
        Gio.DBusError.strip_remote_error(err);
        console.warn(`queue-focus: ${name} failed: ${err.message}`);
        Main.notifyError(APP_NAME, err.message);
    }

    /** Complete the current task; the notification offers to undo that completion. */
    completeCurrent() {
        this._proxy?.CompleteCurrentRemote((res, err) => {
            if (err) {
                this._fail('CompleteCurrent', err);
                return;
            }
            const [id, title] = res;
            if (!id) {
                this._notify('Nothing in Now');
                return;
            }
            // Only the latest completion can be undone: retire the earlier offer.
            this._doneNotification?.destroy();
            this._doneNotification = this._notify('Done', title,
                {label: 'Undo', activate: () => this._undoComplete(id)});
            this._doneNotification.connect('destroy', notification => {
                if (this._doneNotification === notification) this._doneNotification = null;
            });
        });
    }

    _undoComplete(id) {
        this._proxy?.UndoCompleteRemote(id, (res, err) => {
            if (err) {
                this._fail('UndoComplete', err);
                return;
            }
            if (!res[0]) this._notify('Nothing to undo', 'The queue changed since.');
        });
    }

    /** Show a transient banner from the extension's own source and return it. */
    _notify(title, body = null, action = null) {
        if (!this._source) {
            this._source = new MessageTray.Source({title: APP_NAME, iconName: APP_ICON});
            this._source.connect('destroy', () => {
                this._source = null;
            });
            Main.messageTray.add(this._source);
        }
        const notification = new MessageTray.Notification({
            source: this._source, title, body, isTransient: true,
        });
        if (action) notification.addAction(action.label, action.activate);
        this._source.addNotification(notification);
        return notification;
    }

    // ---- menu -------------------------------------------------------------

    /** Remember `actor` under `key` so a rebuild can hand focus back to it. */
    _focusable(key, actor) {
        this._focusTargets.set(key, actor);
        return actor;
    }

    /** The focus target holding key focus, or the one a pending restore is about to. */
    _focusedKey() {
        if (this._focusId) return this._focusKey;
        const focus = global.stage.get_key_focus();
        if (!focus) return null;
        for (const [key, actor] of this._focusTargets) {
            if (actor.contains(focus)) return key;
        }
        return null;
    }

    /**
     * Give key focus to a target once the menu has settled (the shell moves
     * focus itself while opening). A target that no longer exists, because
     * its task went away, falls back to the entry.
     */
    _restoreFocus(key) {
        this._cancelFocus();
        if (!key) return;
        this._focusKey = key;
        this._focusId = GLib.idle_add(GLib.PRIORITY_DEFAULT, () => {
            this._focusId = 0;
            (this._focusTargets.get(key) ?? this._entry)?.grab_key_focus();
            return GLib.SOURCE_REMOVE;
        });
    }

    _cancelFocus() {
        if (!this._focusId) return;
        GLib.source_remove(this._focusId);
        this._focusId = 0;
    }

    _taskItem(task, hint, onActivate) {
        const item = new PopupMenu.PopupBaseMenuItem();
        if (task.tag) {
            const chip = new St.Label({
                text: task.tag === 'work' ? 'W' : 'P',
                style_class: `qf-chip qf-chip-${task.tag}`,
                y_align: Clutter.ActorAlign.CENTER,
            });
            item.add_child(chip);
        }
        const label = new St.Label({text: task.title, x_expand: true, y_align: Clutter.ActorAlign.CENTER});
        label.clutter_text.ellipsize = Pango.EllipsizeMode.END;
        item.add_child(label);
        item.add_child(new St.Label({text: hint, style_class: 'qf-hint', y_align: Clutter.ActorAlign.CENTER}));
        item.connect('activate', onActivate);
        return this._focusable(`task:${task.id}`, item);
    }

    _promoteItem(task) {
        return this._taskItem(task, '↑', () => this.call('Promote', task.id));
    }

    _section(title) {
        const item = new PopupMenu.PopupMenuItem(title, {reactive: false, can_focus: false});
        item.label.add_style_class_name('qf-section');
        return item;
    }

    _buildMenu(fresh = false) {
        // A rebuild while the menu is open (a Changed signal from another client)
        // must not eat what the user is doing: carry the quick-add draft over
        // and put key focus back where it was. A fresh open focuses the entry.
        const focusKey = fresh ? 'entry' : this._focusedKey();
        const draft = this._entry?.get_text() ?? '';
        this._focusTargets = new Map();
        this._entry = null;
        this.menu.removeAll();
        const st = this._state;

        const entryItem = new PopupMenu.PopupBaseMenuItem({reactive: false, can_focus: false});
        const entry = new St.Entry({
            hint_text: 'Add…  !now  #w #p  @later @side',
            text: draft,
            x_expand: true,
            style_class: 'qf-entry',
            can_focus: true,
        });
        entry.clutter_text.connect('activate', () => {
            if (this._quickAddPending) return;
            const text = entry.get_text().trim();
            if (!text) return;
            this._quickAddPending = true;
            // An empty bucket means "wherever Settings says".
            this._proxy?.AddRemote(text, '', (_res, err) => {
                this._quickAddPending = false;
                if (err) {
                    this._fail('Add', err);
                    return;
                }
                entry.set_text('');
                this.menu.close();
            });
        });
        entryItem.add_child(entry);
        this.menu.addMenuItem(entryItem);
        this._entry = this._focusable('entry', entry);

        if (!st) {
            const start = new PopupMenu.PopupMenuItem('service not running — click to start');
            start.connect('activate', () => this._refresh());
            this.menu.addMenuItem(this._focusable('start', start));
            this.menu.addMenuItem(this._openItems());
            this._restoreFocus(focusKey);
            return;
        }

        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
        if (st.current) {
            this.menu.addMenuItem(this._section('NOW'));
            this.menu.addMenuItem(this._taskItem(st.current, '✓ done', () => this.completeCurrent()));
            for (const t of st.now.slice(1)) this.menu.addMenuItem(this._promoteItem(t));
        } else {
            const pickable = st.side.length > 0 || st.next.length > 0;
            this.menu.addMenuItem(this._section(pickable ? 'NOW — nothing. Pick one:' : 'NOW — nothing. Add one:'));
        }
        if (st.side.length) {
            this.menu.addMenuItem(this._section('SIDE'));
            for (const t of st.side) this.menu.addMenuItem(this._promoteItem(t));
        }
        if (st.next.length) {
            this.menu.addMenuItem(this._section('NEXT'));
            for (const t of st.next.slice(0, NEXT_PREVIEW)) this.menu.addMenuItem(this._promoteItem(t));
            if (st.next.length > NEXT_PREVIEW) {
                this.menu.addMenuItem(new PopupMenu.PopupMenuItem(
                    `… ${st.next.length - NEXT_PREVIEW} more`, {reactive: false, can_focus: false}));
            }
        }
        if (st.later.length) this.menu.addMenuItem(this._section(`LATER · ${st.later.length}`));

        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
        this.menu.addMenuItem(this._openItems());
        this._restoreFocus(focusKey);
    }

    _openItems() {
        const item = new PopupMenu.PopupBaseMenuItem({reactive: false, can_focus: false});
        const mk = (label, view) => {
            const b = new St.Button({label, style_class: 'button qf-open-btn', x_expand: true, can_focus: true});
            b.connect('clicked', () => {
                this.call('Show', view);
                this.menu.close();
            });
            return this._focusable(`open:${view}`, b);
        };
        item.add_child(mk('Queue', 'queue'));
        item.add_child(mk('Board', 'board'));
        return item;
    }

    destroy() {
        this._flash.destroy();
        this._cancelTick();
        this._cancelRetry();
        this._cancelFocus();
        if (this._proxy) {
            for (const id of this._signalIds) this._proxy.disconnectSignal(id);
            this._proxy.disconnect(this._ownerId);
            this._proxy = null;
        }
        this._source?.destroy();
        this._entry = null;
        this._focusTargets.clear();
        super.destroy();
    }
});

export default class QueueFocusExtension extends Extension {
    enable() {
        this._indicator = new Indicator();
        Main.panel.addToStatusArea(this.uuid, this._indicator, 0, 'center');

        // Global shortcuts, configurable via the extension's gsettings schema.
        this._settings = this.getSettings();
        for (const [name, action] of Object.entries(KEYBINDINGS)) {
            Main.wm.addKeybinding(name, this._settings,
                Meta.KeyBindingFlags.IGNORE_AUTOREPEAT,
                Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW,
                () => {
                    if (this._indicator) action(this._indicator);
                });
        }
    }

    disable() {
        for (const name of Object.keys(KEYBINDINGS)) Main.wm.removeKeybinding(name);
        this._settings = null;
        this._indicator?.destroy();
        this._indicator = null;
    }
}
