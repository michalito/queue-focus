// Queue Focus — top-bar indicator talking to the queue-focus service over D-Bus.
//
// The panel shows the current task: a tag dot, the title, and its clock as a
// pill that pauses the clock when clicked. The menu is a focus card for that
// task on the left and what runs beside it on the right: Side. Next and Later
// are left to the Queue and Board views.
//
// Titles are shown whole, on one line: the panel button and the menu grow
// sideways to fit them, and stop only where the screen does (see layout.js).
import Atk from 'gi://Atk';
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
import {menuMaxWidth, panelTitleRoom} from './layout.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as MessageTray from 'resource:///org/gnome/shell/ui/messageTray.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

const APP_NAME = 'Queue Focus';
const APP_ICON = 'org.queuefocus.QueueFocus-symbolic';
// How long the menu offers to undo a completion made from it.
const UNDO_MS = 8000;
const PAUSE_GLYPH = '❚❚';
// The air kept between the panel's centre box and the boxes either side of it.
const PANEL_GAP = 12;
// gschema key → what to do when pressed.
const KEYBINDINGS = {
    'toggle-queue': ind => ind.call('Show', 'toggle'),
    'quick-add': ind => ind.call('Show', 'add'),
    'show-board': ind => ind.call('Show', 'board'),
    'complete-current': ind => ind.completeCurrent(),
};
const {CENTER, END} = Clutter.ActorAlign;

/** "12m" or "1h02" on the clock; a paused task keeps the time it had. */
function elapsed(startedAt, pausedAt) {
    const s = Math.max(0, (pausedAt || Math.floor(Date.now() / 1000)) - startedAt);
    const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60);
    return h > 0 ? `${h}h${String(m).padStart(2, '0')}` : `${m}m`;
}

/** The clock as the top bar shows it: the pause glyph leads. */
function panelClock(task) {
    const t = elapsed(task.started_at, task.paused_at);
    return task.paused_at ? `${PAUSE_GLYPH} ${t}` : t;
}

/** The clock as the menu's card shows it: the pause glyph trails. */
function cardClock(task) {
    const t = elapsed(task.started_at, task.paused_at);
    return task.paused_at ? `${t} ${PAUSE_GLYPH}` : t;
}

function label(text, styleClass, props = {}) {
    return new St.Label({text, style_class: styleClass, y_align: CENTER, ...props});
}

/**
 * A task title: one line, as wide as its text. Whatever holds it grows to fit,
 * so the ellipsis is only for a title the screen itself has no room for.
 */
function oneLine(actor) {
    actor.clutter_text.ellipsize = Pango.EllipsizeMode.END;
    return actor;
}

function button(text, styleClass, onClick, props = {}) {
    const b = new St.Button({label: text, style_class: styleClass, can_focus: true, ...props});
    b.connect('clicked', onClick);
    return b;
}

function chip(tag) {
    return label(tag === 'work' ? 'W' : 'P', `qf-chip qf-chip-${tag}`);
}

/** Shell 48 still spells the orientation as a boolean. */
function column(styleClass, props = {}) {
    const box = new St.BoxLayout({style_class: styleClass, ...props});
    if ('orientation' in box) box.orientation = Clutter.Orientation.VERTICAL;
    else box.vertical = true;
    return box;
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
        // The latest completion, while it can still be undone from here:
        // `{id, title}` for UNDO_MS, and the notification offering it when
        // the menu was closed at the time.
        this._undo = null;
        this._undoId = 0;
        this._doneNotification = null;
        // The card's clock in the menu currently built, if the card has one.
        this._cardClock = null;
        // A pending refit of the title (see _fitTitle), and its last result.
        this._fitId = 0;
        this._titleRoom = null;

        const box = new St.BoxLayout({style_class: 'qf-box'});
        this._dot = label('●', 'qf-dot qf-dot-none');
        this._label = oneLine(label('…', 'qf-label'));
        this._clock = label('', 'qf-clock');
        // The clock is a pause button living inside the panel button. It takes
        // the press itself so the panel button never turns it into a menu
        // toggle. A plain reactive bin rather than an St.Button, whose own
        // click handling differs between Shell 48 and 50.
        this._pill = new St.Bin({
            style_class: 'qf-pill',
            child: this._clock,
            reactive: true,
            track_hover: true,
            y_align: CENTER,
            accessible_role: Atk.Role.PUSH_BUTTON,
            accessible_name: 'Pause',
        });
        this._pill.connect('button-press-event', (_actor, event) => {
            if (event.get_button() !== Clutter.BUTTON_PRIMARY) return Clutter.EVENT_PROPAGATE;
            this.togglePause();
            return Clutter.EVENT_STOP;
        });
        this._pill.connect('touch-event', (_actor, event) => {
            if (event.type() !== Clutter.EventType.TOUCH_BEGIN) return Clutter.EVENT_PROPAGATE;
            this.togglePause();
            return Clutter.EVENT_STOP;
        });
        box.add_child(this._dot);
        box.add_child(this._label);
        box.add_child(this._pill);
        this.add_child(box);

        this._connection = connectQueue({
            state: state => this._apply(state),
            settings: settings => this._applySettings(settings),
            flash: event => this._flash.show(event),
            warning: message => Main.notifyError(APP_NAME, message),
        });

        // What the title may grow into changes with the panel around it: its
        // width, what either side asks for, and what else shares the centre.
        Main.panel.connectObject('notify::width', () => this._queueFit(), this);
        for (const box of [Main.panel._leftBox, Main.panel._centerBox, Main.panel._rightBox])
            box?.connectObject('notify::allocation', () => this._queueFit(), this);
        global.display.connectObject('workareas-changed', () => this._queueFit(), this);
        // Nor is there a centre box to measure until the button is in it.
        this.container.connectObject('parent-set', () => this._queueFit(), this);

        this.menu.connect('open-state-changed', (_m, open) => {
            if (open) {
                this._capMenu();
                if (!this._quickAddPending) this._buildMenu(true);
                return;
            }
            // A closed menu's clock is nobody's business until it reopens.
            this._cardClock = null;
            this._updateClocks();
        });
        // PopupMenu refuses to open while empty, so populate it before the
        // first open-state-changed signal can ever arrive.
        this._buildMenu();
    }

    // ---- state → panel ----------------------------------------------------

    _apply(state) {
        this._state = state;
        const cur = state?.current ?? null;
        this._dot.style_class = `qf-dot qf-dot-${cur?.tag ?? 'none'}`;
        this._label.text = cur ? cur.title : (state ? 'no task' : 'queue-focus');
        // A rebuilt menu repaints the clocks itself.
        if (!this._refreshMenu()) this._updateClocks();
    }

    _applySettings(settings) {
        this._prefs = settings;
        this._updateClocks();
    }

    /** Show the clocks, then wake up right after their next minute boundary. */
    _updateClocks() {
        this._cancelTick();
        const cur = this._state?.current;
        const timed = !!cur?.started_at;
        const paused = timed && !!cur.paused_at;
        const showing = timed && this._prefs.show_timer !== false;
        this._clock.text = showing ? panelClock(cur) : '';
        // An empty pill still carries its padding, so take it out of the box.
        this._pill.visible = showing;
        this._pill.style_class = paused ? 'qf-pill qf-pill-paused' : 'qf-pill';
        this._pill.accessible_name = paused ? 'Resume' : 'Pause';
        this._label.style_class = paused ? 'qf-label qf-label-paused' : 'qf-label';
        if (this._cardClock) this._cardClock.text = timed ? cardClock(cur) : '';
        // The title and the pill share the button, and both have just changed.
        this._fitTitle();
        if (!timed || paused || (!showing && !this._cardClock)) return;
        const secs = Math.max(0, Math.floor(Date.now() / 1000) - cur.started_at);
        this._tickId = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 60 - (secs % 60), () => {
            this._tickId = 0;
            this._updateClocks();
            return GLib.SOURCE_REMOVE;
        });
    }

    _cancelTick() {
        if (!this._tickId) return;
        GLib.source_remove(this._tickId);
        this._tickId = 0;
    }

    // ---- widths -----------------------------------------------------------

    /**
     * Let the title take the room the panel really has. The panel gives its
     * centre box whatever width it asks for and cuts the side boxes short to
     * pay for it, so an unbounded title would push the system menu off the
     * panel. The limit is therefore the panel's own: see panelTitleRoom.
     */
    _fitTitle() {
        const {_leftBox: left, _centerBox: center, _rightBox: right} = Main.panel;
        const monitor = Main.layoutManager.findMonitorForActor(Main.panel) ?? Main.layoutManager.primaryMonitor;
        // Without these the stylesheet's own limit stands.
        if (!left || !right || !monitor || this.container.get_parent() !== center) return;
        const natural = actor => actor.get_preferred_width(-1)[1];
        const workArea = Main.layoutManager.getWorkAreaForMonitor(monitor.index);
        const rtl = Main.panel.get_text_direction() === Clutter.TextDirection.RTL;
        const room = panelTitleRoom({
            panelWidth: Main.panel.width || monitor.width,
            centerOffset: 2 * (workArea.x - monitor.x) + workArea.width - monitor.width,
            startWidth: natural(rtl ? right : left),
            endWidth: natural(rtl ? left : right),
            centerWidth: natural(center),
            titleWidth: natural(this._label),
            gap: PANEL_GAP,
            scale: St.ThemeContext.get_for_stage(global.stage).scale_factor,
        });
        if (room === this._titleRoom) return;
        this._titleRoom = room;
        this._label.style = `max-width: ${room}px;`;
    }

    /** Refit once the layout pass that called for it is over. */
    _queueFit() {
        if (this._fitId) return;
        const laters = global.compositor.get_laters();
        this._fitId = laters.add(Meta.LaterType.BEFORE_REDRAW, () => {
            this._fitId = 0;
            this._fitTitle();
            return GLib.SOURCE_REMOVE;
        });
    }

    _cancelFit() {
        if (!this._fitId) return;
        global.compositor.get_laters().remove(this._fitId);
        this._fitId = 0;
    }

    /**
     * Keep the menu on the screen. The panel button limits the menu's height
     * on every open, replacing its style, so the width goes in after it.
     */
    _capMenu() {
        const actor = this.menu.actor;
        const workArea = Main.layoutManager.getWorkAreaForMonitor(Main.layoutManager.primaryIndex);
        const width = menuMaxWidth({
            workAreaWidth: workArea.width,
            // The shell keeps a menu this far from the work area's edges.
            edge: actor.get_theme_node().get_length('-arrow-rise'),
            margins: actor.margin_left + actor.margin_right,
            scale: St.ThemeContext.get_for_stage(global.stage).scale_factor,
        });
        actor.style = `${actor.style ?? ''} max-width: ${width}px;`;
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

    /** Pause or resume the current task's clock. */
    togglePause() {
        this._connection.request('TogglePause', [], (res, err) => {
            if (err) {
                this._fail('TogglePause', err);
                return;
            }
            if (res[0]) {
                this._dropUndo();
                this._refreshMenu();
            }
        });
    }

    /** Make a listed task the current one. */
    promote(task) {
        this._connection.request('Promote', [task.id], (_res, err) => {
            if (err) {
                this._fail('Promote', err);
                return;
            }
            this._dropUndo();
            this._refreshMenu();
        });
    }

    /** Complete the current task, then offer to undo that. */
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
            this._offerUndo(id, title);
        });
    }

    /** Mark a listed task done, then offer to undo that. */
    completeTask(task) {
        this._connection.request('Complete', [task.id], (_res, err) => {
            if (err) {
                this._fail('Complete', err);
                return;
            }
            this._offerUndo(task.id, task.title);
        });
    }

    /**
     * Offer to undo the completion just made. The menu shows the offer while
     * it is open; a completion made from a shortcut is offered where the user
     * is looking instead, in a notification. Only the latest completion can
     * be undone, so an earlier offer is retired first.
     */
    _offerUndo(id, title) {
        this._dropUndo();
        this._undo = {id, title};
        this._undoId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, UNDO_MS, () => {
            this._undoId = 0;
            this._undo = null;
            this._refreshMenu();
            return GLib.SOURCE_REMOVE;
        });
        // A pending quick-add holds the menu steady; use a notification when
        // it prevents the inline offer from being rendered.
        if (this.menu.isOpen && this._refreshMenu()) return;
        this._doneNotification = this._notify('Done', title,
            {label: 'Undo', activate: () => this.undoComplete(id)});
        this._doneNotification.connect('destroy', notification => {
            if (this._doneNotification === notification) this._doneNotification = null;
        });
    }

    /**
     * Withdraw the offer: it was taken, it timed out, or the queue changed
     * again from here, which the service would refuse to undo across anyway.
     */
    _dropUndo() {
        if (this._undoId) {
            GLib.source_remove(this._undoId);
            this._undoId = 0;
        }
        this._undo = null;
        this._doneNotification?.destroy();
    }

    undoComplete(id) {
        this._connection.request('UndoComplete', [id], (res, err) => {
            if (err) {
                this._fail('UndoComplete', err);
                return;
            }
            if (!res[0]) this._notify('Nothing to undo', 'The queue changed since.');
            this._dropUndo();
            this._refreshMenu();
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

    /** Rebuild the open menu after the queue, or the undo on offer, changed. */
    _refreshMenu() {
        if (!this.menu.isOpen || this._quickAddPending) return false;
        this._buildMenu();
        return true;
    }

    _buildMenu(fresh = false) {
        // A rebuild while the menu is open (a Changed signal from another client)
        // must not eat what the user is doing: carry the quick-add draft over
        // and put key focus back where it was. A fresh open focuses the entry.
        const focusKey = fresh ? 'entry' : this._focusedKey();
        const draft = this._entry?.get_text() ?? '';
        this._focusTargets = new Map();
        this._entry = null;
        this._cardClock = null;
        this.menu.removeAll();

        // One inert item holds the whole layout; the menu's own keyboard
        // navigation still works between the focusable actors inside it.
        const item = new PopupMenu.PopupBaseMenuItem({reactive: false, can_focus: false, style_class: 'qf-menu'});
        const columns = new St.BoxLayout({style_class: 'qf-columns', x_expand: true});
        columns.add_child(this._focusColumn());
        columns.add_child(this._queueColumn(draft));
        item.add_child(columns);
        this.menu.addMenuItem(item);
        this._updateClocks();
        this._restoreFocus(focusKey);
    }

    /** Left: the current task as a card, its actions, and the view buttons. */
    _focusColumn() {
        const st = this._state;
        const cur = st?.current ?? null;
        const accent = `qf-accent-${cur?.tag ?? 'none'}`;
        const paused = !!cur?.paused_at;
        const col = column('qf-left');
        col.add_child(this._card(cur, accent, paused));

        if (cur) {
            const actions = new St.BoxLayout({style_class: 'qf-actions'});
            actions.add_child(this._focusable('done',
                button('✓ Done', `qf-btn qf-primary ${accent}`, () => this.completeCurrent(), {x_expand: true})));
            actions.add_child(this._focusable('pause',
                button(paused ? '▶ Resume' : `${PAUSE_GLYPH} Pause`, 'qf-btn qf-secondary',
                    () => this.togglePause(), {x_expand: true})));
            col.add_child(actions);
        } else if (!st) {
            col.add_child(this._focusable('start',
                button('Start the service', 'qf-btn qf-secondary', () => this._connection.refresh())));
        }

        if (this._undo) {
            const {id, title} = this._undo;
            const row = new St.BoxLayout({style_class: 'qf-undo'});
            row.add_child(oneLine(label(`Done · ${title}`, 'qf-undo-text', {x_expand: true})));
            row.add_child(this._focusable('undo', button('↶ Undo', 'qf-undo-btn', () => this.undoComplete(id))));
            col.add_child(row);
        }

        col.add_child(this._footer());
        return col;
    }

    /** The current task, washed with its tag's accent: NOW, title, clock. */
    _card(cur, accent, paused) {
        const card = column(`qf-card ${accent}`);
        const head = new St.BoxLayout({style_class: 'qf-card-head'});
        head.add_child(label('NOW', 'qf-section'));
        if (cur?.tag) head.add_child(chip(cur.tag));
        if (paused) head.add_child(label('PAUSED', 'qf-paused', {x_expand: true, x_align: END}));
        card.add_child(head);
        if (!cur) {
            const hint = this._state ? 'Nothing in Now.\nPick one from Side →\nor add one with !' : 'The service is not running.';
            card.add_child(label(hint, 'qf-card-empty', {y_expand: true}));
            return card;
        }
        card.add_child(oneLine(new St.Label({text: cur.title, style_class: 'qf-card-title'})));
        if (cur.started_at) {
            this._cardClock = label('', `qf-card-clock${paused ? ' qf-card-clock-paused' : ''}`,
                {y_expand: true, y_align: END});
            card.add_child(this._cardClock);
        }
        return card;
    }

    /** The view buttons along the bottom of the left column. */
    _footer() {
        const row = new St.BoxLayout({style_class: 'qf-footer', y_expand: true, y_align: END});
        const open = (text, view, styleClass = 'qf-open-btn', props = {}) => this._focusable(`open:${view}`,
            button(text, styleClass, () => {
                this.call('Show', view);
                this.menu.close();
            }, props));
        row.add_child(open('Queue', 'queue'));
        row.add_child(open('Board', 'board'));
        row.add_child(open('⚙', 'settings', 'qf-open-btn qf-gear',
            {x_expand: true, x_align: END, accessible_name: 'Settings'}));
        return row;
    }

    /** Right: the quick-add entry, then Side. */
    _queueColumn(draft) {
        const st = this._state;
        const cur = st?.current ?? null;
        const col = column('qf-right', {x_expand: true});
        col.add_child(this._quickAdd(draft));

        const scroll = new St.ScrollView({
            style_class: 'qf-scroll',
            hscrollbar_policy: St.PolicyType.NEVER,
            vscrollbar_policy: St.PolicyType.AUTOMATIC,
            overlay_scrollbars: true,
            x_expand: true,
        });
        // Side cards take a wash of the current task's accent, like the card.
        const list = column(`qf-list qf-accent-${cur?.tag ?? 'none'}`, {x_expand: true});
        scroll.child = list;
        col.add_child(scroll);
        if (!st) return col;

        list.add_child(label('SIDE', 'qf-section qf-list-head'));
        const side = column('qf-side');
        for (const t of st.side) side.add_child(this._sideCard(t));
        if (!st.side.length) {
            const empty = label('Nothing on the side.\nAdd one with @side.', 'qf-side-empty-text',
                {x_align: CENTER});
            empty.clutter_text.line_alignment = Pango.Alignment.CENTER;
            side.add_child(new St.Bin({style_class: 'qf-side-empty', child: empty, x_expand: true}));
        }
        list.add_child(side);
        return col;
    }

    _quickAdd(draft) {
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
                this._dropUndo();
                entry.set_text('');
                this.menu.close();
            });
        });
        this._entry = this._focusable('entry', entry);
        return entry;
    }

    /**
     * A Side task with its actions, shown while the pointer or key focus is on
     * it. Hidden, they still take their room, so revealing them moves nothing.
     */
    _sideCard(task) {
        const row = new St.BoxLayout({style_class: 'qf-side-card', reactive: true, track_hover: true});
        if (task.tag) row.add_child(chip(task.tag));
        row.add_child(oneLine(label(task.title, 'qf-side-title', {x_expand: true})));
        const actions = new St.BoxLayout({style_class: 'qf-side-actions', y_align: CENTER, opacity: 0});
        const promote = button('↑', 'qf-act', () => this.promote(task), {accessible_name: 'Make current'});
        const done = button('✓', 'qf-act', () => this.completeTask(task), {accessible_name: 'Done'});
        actions.add_child(this._focusable(`task:${task.id}:promote`, promote));
        actions.add_child(this._focusable(`task:${task.id}:done`, done));
        row.add_child(actions);
        const reveal = () => {
            actions.opacity = row.hover || promote.has_key_focus() || done.has_key_focus() ? 255 : 0;
        };
        row.connect('notify::hover', reveal);
        for (const b of [promote, done]) {
            b.connect('key-focus-in', reveal);
            b.connect('key-focus-out', reveal);
        }
        return row;
    }

    destroy() {
        this._flash.destroy();
        this._cancelTick();
        this._cancelFit();
        this._dropUndo();
        this._connection.destroy();
        this._cancelFocus();
        this._source?.destroy();
        this._entry = null;
        this._cardClock = null;
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
