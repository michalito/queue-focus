// Queue Focus — top-bar indicator talking to the queue-focus service over D-Bus.
import GObject from 'gi://GObject';
import GLib from 'gi://GLib';
import St from 'gi://St';
import Clutter from 'gi://Clutter';
import Meta from 'gi://Meta';
import Pango from 'gi://Pango';
import Shell from 'gi://Shell';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import {FlashOverlay} from './flash.js';
import {connectQueue} from './dbus.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as MessageTray from 'resource:///org/gnome/shell/ui/messageTray.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

const APP_NAME = 'Queue Focus';
const APP_ICON = 'org.queuefocus.QueueFocus-symbolic';
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

        this._connection = connectQueue({
            state: state => this._apply(state),
            settings: settings => this._applySettings(settings),
            flash: event => this._flash.show(event),
            warning: message => Main.notifyError(APP_NAME, message),
        });

        this.menu.connect('open-state-changed', (_m, open) => {
            if (open && !this._quickAddPending) this._buildMenu(true);
        });
        // PopupMenu refuses to open while empty, so populate it before the
        // first open-state-changed signal can ever arrive.
        this._buildMenu();
    }

    // ---- state → panel ----------------------------------------------------

    _apply(state) {
        this._state = state;
        const cur = this._state?.current ?? null;
        for (const c of ['qf-dot-work', 'qf-dot-personal', 'qf-dot-none']) this._dot.remove_style_class_name(c);
        this._dot.add_style_class_name(cur?.tag ? `qf-dot-${cur.tag}` : 'qf-dot-none');
        this._label.text = cur ? cur.title : (this._state ? 'no task' : 'queue-focus');
        this._updateTimer();
        if (this.menu.isOpen && !this._quickAddPending) this._buildMenu();
    }

    _applySettings(settings) {
        this._prefs = settings;
        this._updateTimer();
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
        this._connection.request(name, args, (_res, err) => {
            if (err) this._fail(name, err);
        });
    }

    _fail(name, err) {
        console.warn(`queue-focus: ${name} failed: ${err.message}`);
        Main.notifyError(APP_NAME, err.message);
    }

    /** Complete the current task; the notification offers to undo that completion. */
    completeCurrent() {
        this._connection.request('CompleteCurrent', [], (res, err) => {
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
        this._connection.request('UndoComplete', [id], (res, err) => {
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
            this._connection.request('Add', [text, ''], (_res, err) => {
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
            start.connect('activate', () => this._connection.refresh());
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
        this._connection.destroy();
        this._cancelFocus();
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
