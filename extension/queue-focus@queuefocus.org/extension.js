// Queue Focus — top-bar indicator talking to the queue-focus service over D-Bus.
import GObject from 'gi://GObject';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';
import Clutter from 'gi://Clutter';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

const BUS_NAME = 'org.queuefocus.QueueFocus';
const OBJ_PATH = '/org/queuefocus/QueueFocus';
const IFACE_XML = `
<node>
  <interface name="org.queuefocus.QueueFocus1">
    <method name="GetState"><arg type="s" name="json" direction="out"/></method>
    <method name="Add"><arg type="s" name="text" direction="in"/><arg type="s" name="bucket" direction="in"/><arg type="t" name="id" direction="out"/></method>
    <method name="CompleteCurrent"><arg type="b" name="completed" direction="out"/></method>
    <method name="Promote"><arg type="t" name="id" direction="in"/></method>
    <method name="Remove"><arg type="t" name="id" direction="in"/></method>
    <method name="Show"><arg type="s" name="view" direction="in"/></method>
    <method name="Hide"/>
    <signal name="Changed"><arg type="s" name="json"/></signal>
  </interface>
</node>`;
const QueueFocusProxy = Gio.DBusProxy.makeProxyWrapper(IFACE_XML);

const MAX_TITLE = 42;
// Reconnect delay after the service goes away (e.g. restarted by an update).
const REACTIVATE_MS = 1500;
// gschema key → what to do when pressed.
const KEYBINDINGS = {
    'toggle-queue': ind => ind.call('Show', 'toggle'),
    'quick-add': ind => ind.call('Show', 'add'),
    'show-board': ind => ind.call('Show', 'board'),
    'complete-current': ind => ind.completeCurrent(),
};

function elapsed(startedAt) {
    if (!startedAt) return '';
    const s = Math.max(0, Math.floor(Date.now() / 1000) - startedAt);
    const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60);
    return h > 0 ? `${h}h${String(m).padStart(2, '0')}` : `${m}m`;
}

function ellipsize(t) {
    return t.length > MAX_TITLE ? t.slice(0, MAX_TITLE - 1) + '…' : t;
}

const Indicator = GObject.registerClass(
class QueueFocusIndicator extends PanelMenu.Button {
    _init() {
        super._init(0.5, 'Queue Focus');
        this._state = null;
        this._quickAddPending = false;

        const box = new St.BoxLayout({style_class: 'qf-box'});
        this._dot = new St.Label({text: '●', style_class: 'qf-dot', y_align: Clutter.ActorAlign.CENTER});
        this._label = new St.Label({text: '…', style_class: 'qf-label', y_align: Clutter.ActorAlign.CENTER});
        this._timer = new St.Label({text: '', style_class: 'qf-timer', y_align: Clutter.ActorAlign.CENTER});
        box.add_child(this._dot);
        box.add_child(this._label);
        box.add_child(this._timer);
        this.add_child(box);

        this._proxy = new QueueFocusProxy(Gio.DBus.session, BUS_NAME, OBJ_PATH, (proxy, error) => {
            if (error) { console.warn(`queue-focus: proxy error: ${error.message}`); return; }
            this._refresh();
        });
        this._signalId = this._proxy.connectSignal('Changed', (_p, _s, [json]) => this._apply(json));
        this._ownerId = this._proxy.connect('notify::g-name-owner', () => {
            if (this._proxy.g_name_owner) { this._refresh(); return; }
            this._apply(null);
            // Service gone (quit / updated): re-activate it shortly.
            if (!this._reactivateId) {
                this._reactivateId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, REACTIVATE_MS, () => {
                    this._reactivateId = 0;
                    this._refresh();
                    return GLib.SOURCE_REMOVE;
                });
            }
        });
        this._tickId = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 30, () => {
            this._updateTimer();
            return GLib.SOURCE_CONTINUE;
        });

        this.menu.connect('open-state-changed', (_m, open) => {
            if (open && !this._quickAddPending) this._buildMenu();
        });
    }

    _refresh() {
        // Calling a method auto-starts the service via D-Bus activation.
        this._proxy.GetStateRemote((res, err) => {
            if (err) { console.warn(`queue-focus: GetState failed: ${err.message}`); this._apply(null); return; }
            this._apply(res[0]);
        });
    }

    _apply(json) {
        try { this._state = json ? JSON.parse(json) : null; } catch (_e) { this._state = null; }
        const cur = this._state?.current ?? null;
        for (const c of ['qf-dot-work', 'qf-dot-personal', 'qf-dot-none']) this._dot.remove_style_class_name(c);
        this._dot.add_style_class_name(cur?.tag ? `qf-dot-${cur.tag}` : 'qf-dot-none');
        this._label.text = cur ? ellipsize(cur.title) : (this._state ? 'no task' : 'queue-focus');
        this._updateTimer();
        if (this.menu.isOpen && !this._quickAddPending) this._buildMenu();
    }

    _updateTimer() {
        const cur = this._state?.current ?? null;
        this._timer.text = cur?.started_at ? `  ${elapsed(cur.started_at)}` : '';
    }

    call(name, ...args) {
        this._proxy[`${name}Remote`](...args, (_res, err) => {
            if (err) console.warn(`queue-focus: ${name} failed: ${err.message}`);
        });
    }

    /** Complete the current task and confirm on screen. */
    completeCurrent() {
        const title = this._state?.current?.title;
        this._proxy.CompleteCurrentRemote(([done], err) => {
            if (err) { console.warn(`queue-focus: CompleteCurrent failed: ${err.message}`); return; }
            const icon = Gio.ThemedIcon.new(done ? 'object-select-symbolic' : 'dialog-information-symbolic');
            Main.osdWindowManager.show(-1, icon, done ? `Done: ${title}` : 'Nothing in Now');
        });
    }

    _taskItem(task, {activateLabel, onActivate, reactive = true} = {}) {
        const item = new PopupMenu.PopupBaseMenuItem({reactive, can_focus: reactive});
        if (task.tag) {
            const chip = new St.Label({text: task.tag === 'work' ? 'W' : 'P', style_class: `qf-chip qf-chip-${task.tag}`, y_align: Clutter.ActorAlign.CENTER});
            item.add_child(chip);
        }
        const label = new St.Label({text: task.title, x_expand: true, y_align: Clutter.ActorAlign.CENTER});
        label.clutter_text.ellipsize = 3; // Pango.EllipsizeMode.END
        item.add_child(label);
        if (activateLabel) {
            const hint = new St.Label({text: activateLabel, style_class: 'qf-hint', y_align: Clutter.ActorAlign.CENTER});
            item.add_child(hint);
        }
        if (onActivate) item.connect('activate', onActivate);
        return item;
    }

    _section(title) {
        const item = new PopupMenu.PopupMenuItem(title, {reactive: false, can_focus: false});
        item.label.add_style_class_name('qf-section');
        return item;
    }

    _buildMenu() {
        this.menu.removeAll();
        const st = this._state;

        // Quick-add entry.
        const entryItem = new PopupMenu.PopupBaseMenuItem({reactive: false, can_focus: false});
        const entry = new St.Entry({hint_text: 'Add…  !now  #w #p  @later @side', x_expand: true, style_class: 'qf-entry', can_focus: true});
        entry.clutter_text.connect('activate', () => {
            if (this._quickAddPending) return;
            const text = entry.get_text().trim();
            if (!text) return;
            this._quickAddPending = true;
            this._proxy.AddRemote(text, 'next', (_res, err) => {
                this._quickAddPending = false;
                if (err) { console.warn(`queue-focus: Add failed: ${err.message}`); return; }
                entry.set_text('');
                this.menu.close();
            });
        });
        entryItem.add_child(entry);
        this.menu.addMenuItem(entryItem);
        GLib.idle_add(GLib.PRIORITY_DEFAULT, () => { entry.grab_key_focus(); return GLib.SOURCE_REMOVE; });

        if (!st) {
            this.menu.addMenuItem(new PopupMenu.PopupMenuItem('service not running — click to start', {}));
            this.menu.addMenuItem(this._openItems());
            return;
        }

        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
        if (st.current) {
            this.menu.addMenuItem(this._section('NOW'));
            this.menu.addMenuItem(this._taskItem(st.current, {activateLabel: '✓ done', onActivate: () => this.call('CompleteCurrent')}));
            for (const t of st.now.slice(1))
                this.menu.addMenuItem(this._taskItem(t, {activateLabel: '↑', onActivate: () => this.call('Promote', t.id)}));
        } else {
            this.menu.addMenuItem(this._section('NOW — nothing. Pick one:'));
        }
        if (st.side.length) {
            this.menu.addMenuItem(this._section('SIDE'));
            for (const t of st.side)
                this.menu.addMenuItem(this._taskItem(t, {activateLabel: '↑', onActivate: () => this.call('Promote', t.id)}));
        }
        if (st.next.length) {
            this.menu.addMenuItem(this._section('NEXT'));
            for (const t of st.next.slice(0, 8))
                this.menu.addMenuItem(this._taskItem(t, {activateLabel: '↑', onActivate: () => this.call('Promote', t.id)}));
            if (st.next.length > 8)
                this.menu.addMenuItem(new PopupMenu.PopupMenuItem(`… ${st.next.length - 8} more`, {reactive: false, can_focus: false}));
        }
        if (st.later.length)
            this.menu.addMenuItem(this._section(`LATER · ${st.later.length}`));

        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
        this.menu.addMenuItem(this._openItems());
    }

    _openItems() {
        const item = new PopupMenu.PopupBaseMenuItem({reactive: false, can_focus: false});
        const mk = (label, view) => {
            const b = new St.Button({label, style_class: 'button qf-open-btn', x_expand: true, can_focus: true});
            b.connect('clicked', () => { this.call('Show', view); this.menu.close(); });
            return b;
        };
        item.add_child(mk('Queue', 'queue'));
        item.add_child(mk('Board', 'board'));
        return item;
    }

    destroy() {
        if (this._tickId) { GLib.source_remove(this._tickId); this._tickId = 0; }
        if (this._reactivateId) { GLib.source_remove(this._reactivateId); this._reactivateId = 0; }
        if (this._proxy) {
            if (this._signalId) this._proxy.disconnectSignal(this._signalId);
            if (this._ownerId) this._proxy.disconnect(this._ownerId);
            this._proxy = null;
        }
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
                () => { if (this._indicator) action(this._indicator); });
        }
    }

    disable() {
        for (const name of Object.keys(KEYBINDINGS)) Main.wm.removeKeybinding(name);
        this._settings = null;
        this._indicator?.destroy();
        this._indicator = null;
    }
}
