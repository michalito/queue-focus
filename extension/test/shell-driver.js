// Test-only extension loaded alongside the shipped extension in a disposable
// GNOME Shell. No production handlers or actors are replaced.
import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import St from 'gi://St';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const UUID = 'queue-focus@queuefocus.org';
const BUS_NAME = 'org.queuefocus.QueueFocus';
const xml = `<node><interface name="org.queuefocus.QueueFocus1">
<method name="GetState"><arg type="s" direction="out"/></method>
<method name="GetSettings"><arg type="s" direction="out"/></method>
<method name="Add"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="t" direction="out"/></method>
<method name="CompleteCurrent"><arg type="t" direction="out"/><arg type="s" direction="out"/></method>
<method name="Complete"><arg type="t" direction="in"/></method>
<method name="UndoComplete"><arg type="t" direction="in"/><arg type="b" direction="out"/></method>
<method name="TogglePause"><arg type="b" direction="out"/></method>
<method name="Promote"><arg type="t" direction="in"/></method>
<method name="Show"><arg type="s" direction="in"/></method>
<signal name="Changed"><arg type="s"/></signal>
<signal name="SettingsChanged"><arg type="s"/></signal>
<signal name="Stopping"/>
</interface></node>`;
const check = (value, message) => { if (!value) throw new Error(message); };
/** Every label under `actor`, in the order they are drawn. */
const labels = actor => actor.get_children().flatMap(c => c instanceof St.Label ? [c] : labels(c));
const unixNow = () => Math.floor(Date.now() / 1000);

/** Just enough of the service's queue to answer the indicator's calls. */
class Queue {
    constructor() {
        this.tasks = [];
        this.nextId = 1;
        this.undo = null;
    }

    add(title, bucket, tag = null) {
        const id = this.nextId++;
        this.tasks.push({id, title, tag, bucket, started_at: null, paused_at: null});
        if (bucket === 'now') this.promote(id);
        this._normalize();
        return id;
    }

    current() { return this.tasks.find(t => t.bucket === 'now') ?? null; }

    complete(id) {
        const index = this.tasks.findIndex(t => t.id === id);
        if (index < 0) return null;
        const [task] = this.tasks.splice(index, 1);
        let pulled = null;
        if (task.bucket === 'now') {
            pulled = this.tasks.find(t => t.bucket === 'next') ?? null;
            if (pulled) pulled.bucket = 'now';
        }
        this._normalize();
        this.undo = {task, index, pulled: pulled?.id ?? null};
        return task;
    }

    undoComplete(id) {
        if (this.undo?.task.id !== id) return false;
        const {task, index, pulled} = this.undo;
        this.undo = null;
        if (pulled) this.tasks.find(t => t.id === pulled).bucket = 'next';
        this.tasks.splice(Math.min(index, this.tasks.length), 0, task);
        this._normalize();
        return true;
    }

    /** Now holds one task: the one it replaces steps back to the front of Next. */
    promote(id) {
        const task = this.tasks.find(t => t.id === id);
        if (!task) return false;
        const was = this.tasks.find(t => t.bucket === 'now' && t !== task);
        task.bucket = 'now';
        if (was) {
            was.bucket = 'next';
            // Ahead of everything stored, so it leads Next.
            this.tasks = [was, ...this.tasks.filter(t => t !== was)];
        }
        this._normalize();
        return true;
    }

    togglePause() {
        const cur = this.current();
        if (!cur?.started_at) return false;
        if (cur.paused_at) {
            cur.started_at += unixNow() - cur.paused_at;
            cur.paused_at = null;
        } else {
            cur.paused_at = unixNow();
        }
        return true;
    }

    /** Only the current task carries a clock, as in the service. */
    _normalize() {
        const cur = this.current();
        for (const t of this.tasks) {
            if (t === cur) {
                t.started_at ??= unixNow();
            } else {
                t.started_at = null;
                t.paused_at = null;
            }
        }
    }

    snapshot() {
        const list = bucket => this.tasks.filter(t => t.bucket === bucket);
        return JSON.stringify({
            current: this.current(), now: list('now'), side: list('side'), next: list('next'), later: list('later'),
        });
    }
}

export default class ShellTest extends Extension {
    enable() {
        this._sources = new Set();
        this._requests = [];
        this._calls = [];
        this._queue = new Queue();
        this._settings = {show_timer: false};
        this._bus = Gio.DBus.session;
        const record = name => (...args) => this._calls.push({name, args});
        this._fixture = Gio.DBusExportedObject.wrapJSObject(xml, {
            GetState: () => this._queue.snapshot(),
            GetSettings: () => JSON.stringify(this._settings),
            AddAsync: (args, invocation) => this._requests.push({args, invocation}),
            CompleteCurrent: () => {
                record('CompleteCurrent')();
                const cur = this._queue.current();
                if (!cur) return [0, ''];
                const done = this._queue.complete(cur.id);
                this._changed();
                return [done.id, done.title];
            },
            Complete: id => {
                record('Complete')(id);
                if (!this._queue.complete(Number(id))) throw new Error('no such task');
                this._changed();
            },
            UndoComplete: id => {
                record('UndoComplete')(id);
                const undone = this._queue.undoComplete(Number(id));
                if (undone) this._changed();
                return undone;
            },
            TogglePause: () => {
                record('TogglePause')();
                if (this._rejectPause) throw new Error('fixture: could not save');
                const toggled = this._queue.togglePause();
                if (toggled) this._changed();
                return toggled;
            },
            Promote: id => {
                record('Promote')(id);
                if (this._rejectPromote) throw new Error('fixture: could not save');
                if (!this._queue.promote(Number(id))) throw new Error('no such task');
                this._changed();
            },
            Show: record('Show'),
        });
        this._fixture.export(this._bus, '/org/queuefocus/QueueFocus');
        this._run().then(() => this._result({ok: true})).catch(error => {
            logError(error);
            this._result({ok: false, error: error.message, stack: error.stack});
        });
    }

    _changed() {
        this._fixture.emit_signal('Changed', new GLib.Variant('(s)', [this._queue.snapshot()]));
    }

    _callsNamed(name) { return this._calls.filter(c => c.name === name); }

    _result(result) {
        GLib.file_set_contents(GLib.getenv('QF_SHELL_TEST_RESULT'), JSON.stringify(result));
    }

    _wait(predicate, description) {
        return new Promise((resolve, reject) => {
            const deadline = GLib.get_monotonic_time() + 10_000_000;
            const id = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 20, () => {
                try {
                    if (predicate()) {
                        this._sources.delete(id);
                        resolve();
                        return GLib.SOURCE_REMOVE;
                    }
                    if (GLib.get_monotonic_time() > deadline) throw new Error(`Timed out: ${description}`);
                } catch (error) {
                    this._sources.delete(id);
                    reject(error);
                    return GLib.SOURCE_REMOVE;
                }
                return GLib.SOURCE_CONTINUE;
            });
            this._sources.add(id);
        });
    }

    _own() {
        return new Promise(resolve => {
            this._ownerId = Gio.bus_own_name_on_connection(this._bus, BUS_NAME,
                Gio.BusNameOwnerFlags.NONE, resolve, null);
        });
    }

    /**
     * Move a virtual pointer over an actor, optionally pressing and releasing.
     * Where the actor is only means something once the layout it is part of
     * has caught up with the change that came before, so wait for that.
     */
    async _pointer(actor, click = false) {
        await this._wait(() => {
            for (let a = actor; a && a !== global.stage; a = a.get_parent())
                if (!a.has_allocation()) return false;
            return true;
        }, 'the actor under the pointer is laid out');
        this._device ??= Clutter.get_default_backend().get_default_seat()
            .create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);
        const [x, y] = actor.get_transformed_position();
        const [w, h] = actor.get_transformed_size();
        this._device.notify_absolute_motion(GLib.get_monotonic_time(), x + w / 2, y + h / 2);
        if (!click) return;
        this._device.notify_button(GLib.get_monotonic_time(), Clutter.BUTTON_PRIMARY, Clutter.ButtonState.PRESSED);
        this._device.notify_button(GLib.get_monotonic_time(), Clutter.BUTTON_PRIMARY, Clutter.ButtonState.RELEASED);
    }

    /** Save the stage as a PNG when the run asks for pictures. */
    async _shot(name) {
        const dir = GLib.getenv('QF_SHELL_TEST_SHOTS');
        if (!dir) return;
        // Let the pending relayout and the hover it may reveal land first.
        await new Promise(resolve => {
            const id = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 300, () => {
                this._sources.delete(id);
                resolve();
                return GLib.SOURCE_REMOVE;
            });
            this._sources.add(id);
        });
        const file = Gio.File.new_for_path(`${dir}/${name}.png`);
        const stream = file.replace(null, false, Gio.FileCreateFlags.NONE, null);
        await new Promise((resolve, reject) => {
            new Shell.Screenshot().screenshot(false, stream, (shooter, res) => {
                try {
                    shooter.screenshot_finish(res);
                    resolve();
                } catch (error) {
                    reject(error);
                }
            });
        });
        stream.close(null);
    }

    async _run() {
        await this._own();
        // Start the actual extension only after the fixture owns its bus name.
        global.settings.set_strv('enabled-extensions', [this.uuid, UUID]);
        await this._wait(() => Main.panel.statusArea[UUID]?._state, 'indicator reads fixture');
        const indicator = Main.panel.statusArea[UUID];
        indicator.menu.open();
        await this._wait(() => indicator._entry, 'quick-add entry exists');
        const draft = 'preserve this draft';
        const entry = indicator._entry;
        entry.set_text(draft);
        entry.clutter_text.emit('activate');
        await this._wait(() => this._requests.length === 1, 'first add reaches fixture');
        check(this._requests[0].args[0] === draft, 'handler submitted the draft');
        entry.clutter_text.emit('activate');

        // Lose the actual bus owner while Add is unresolved. The handler must
        // release pending state, then carry the draft into the rebuilt menu.
        this._fixture.emit_signal('Stopping', null);
        this._bus.flush_sync(null);
        Gio.bus_unown_name(this._ownerId);
        this._ownerId = null;
        await this._wait(() => indicator._state === null && indicator._entry !== entry,
            'owner loss rebuilds the menu');
        check(indicator._entry.get_text() === draft, 'disconnect retained draft in rebuilt entry');
        check(this._requests.length === 1, 'pending Enter did not submit twice');
        check(indicator._focusTargets.has('start'), 'disconnected menu offers to start the service');

        await this._own();
        await this._wait(() => indicator._state !== null, 'indicator reconnects');
        this._requests[0].invocation.return_value(new GLib.Variant('(t)', [1]));
        this._changed();
        const beforeReply = indicator._entry;
        await this._wait(() => indicator._entry !== beforeReply, 'late reply and fresh change drain');
        check(indicator._entry.get_text() === draft, 'obsolete success did not clear draft');
        check(indicator.menu.isOpen, 'obsolete success did not close menu');

        // A new explicit submission succeeds and clears the real entry.
        indicator._entry.clutter_text.emit('activate');
        await this._wait(() => this._requests.length === 2, 'pending state released for explicit retry');
        this._requests[1].invocation.return_value(new GLib.Variant('(t)', [2]));
        await this._wait(() => !indicator.menu.isOpen, 'successful add closes menu');
        indicator.menu.open();
        check(indicator._entry.get_text() === '', 'successful add clears draft');

        // Disable while another request is pending, then deliver its reply.
        indicator._entry.set_text('discard with extension');
        indicator._entry.clutter_text.emit('activate');
        await this._wait(() => this._requests.length === 3, 'last add pending');
        global.settings.set_strv('enabled-extensions', [this.uuid]);
        await this._wait(() => !Main.panel.statusArea[UUID], 'extension disabled');
        this._requests[2].invocation.return_value(new GLib.Variant('(t)', [3]));
        global.settings.set_strv('enabled-extensions', [this.uuid, UUID]);
        await this._wait(() => Main.panel.statusArea[UUID]?._state, 'extension enables cleanly again');
        check(this._requests.length === 3, 'no automatic add replay');

        await this._menu(Main.panel.statusArea[UUID]);
        await this._widths(Main.panel.statusArea[UUID]);
    }

    /** The focus card and queue with a populated fixture, driven by pointer and actors. */
    async _menu(indicator) {
        const q = this._queue;
        const fix = q.add('Fix login redirect loop', 'now', 'work');
        const notes = q.add('Write release notes 0.2.0', 'next', 'work');
        const ci = q.add('CI run for main', 'side', 'work');
        const landlord = q.add('Wait for landlord reply', 'side', 'personal');
        q.add("Review a colleague's PR", 'next', 'work');
        q.current().started_at = unixNow() - 47 * 60;
        this._settings = {show_timer: true};
        this._fixture.emit_signal('SettingsChanged', new GLib.Variant('(s)', [JSON.stringify(this._settings)]));
        this._changed();
        await this._wait(() => indicator._pill.visible && indicator._clock.text === '47m', 'panel shows the clock');
        check(indicator._label.text === 'Fix login redirect loop', 'panel shows the current title');

        // The clock pill pauses the clock instead of opening the menu.
        check(!indicator.menu.isOpen, 'menu starts closed');
        await this._pointer(indicator._pill, true);
        await this._wait(() => this._callsNamed('TogglePause').length === 1, 'pill press reaches TogglePause');
        await this._wait(() => indicator._clock.text === '❚❚ 47m', 'panel shows the paused clock');
        check(!indicator.menu.isOpen, 'pill press did not open the menu');
        check(indicator._pill.has_style_class_name('qf-pill-paused'), 'paused pill is styled as such');

        // The rest of the panel button opens the menu as before.
        await this._pointer(indicator._label, true);
        await this._wait(() => indicator.menu.isOpen, 'title press opens the menu');
        await this._wait(() => indicator._cardClock, 'card shows a clock');
        check(indicator._cardClock.text === '47m ❚❚', 'card clock trails the pause glyph');
        const targets = () => indicator._focusTargets;
        for (const key of ['entry', 'done', 'pause', `task:${ci}:promote`, `task:${ci}:done`,
            `task:${landlord}:promote`, 'open:queue', 'open:board', 'open:settings']) {
            check(targets().has(key), `menu offers ${key}`);
        }
        check(!targets().has(`task:${fix}:done`), 'the current task is not listed again');
        check(!targets().has(`task:${notes}:done`), 'Next is left to the Queue and Board views');
        const headings = labels(indicator.menu.actor).map(l => l.text).filter(t => /^[A-Z ]{2,}$/.test(t));
        check(headings.join() === 'NOW,PAUSED,SIDE', `the menu lists Now as one task, then Side: ${headings}`);
        const sideRow = targets().get(`task:${ci}:done`).get_parent().get_parent();
        await this._pointer(sideRow);
        await this._wait(() => sideRow.hover, 'pointer hovers the side card');
        check(targets().get(`task:${ci}:done`).get_parent().opacity === 255, 'hover reveals the actions');
        await this._shot('menu-paused');

        // Resume from the menu.
        targets().get('pause').emit('clicked', 1);
        await this._wait(() => this._callsNamed('TogglePause').length === 2, 'pause button reaches TogglePause');
        await this._wait(() => indicator._cardClock?.text === '47m', 'card clock resumes');

        // A pending Add freezes the menu, so completion must offer a notification.
        const pendingEntry = indicator._entry;
        const addCount = this._requests.length;
        pendingEntry.set_text('slow add');
        pendingEntry.clutter_text.emit('activate');
        await this._wait(() => this._requests.length === addCount + 1, 'slow add reaches fixture');
        targets().get(`task:${ci}:done`).emit('clicked', 1);
        await this._wait(() => indicator._undo?.id === ci && indicator._doneNotification,
            'completion during pending add offers undo notification');
        check(indicator._entry === pendingEntry, 'pending add keeps its entry');
        check(!targets().has('undo'), 'frozen menu cannot show inline undo');
        indicator.undoComplete(ci);
        await this._wait(() => !indicator._undo && q.tasks.some(t => t.id === ci),
            'undo during pending add restores the task');
        this._requests[addCount].invocation.return_value(new GLib.Variant('(t)', [99]));
        await this._wait(() => !indicator.menu.isOpen, 'slow add finishes');
        indicator.menu.open();

        // Done on a Side card: the menu offers to undo, no notification.
        targets().get(`task:${ci}:done`).emit('clicked', 1);
        await this._wait(() => indicator._undo?.id === ci && targets().has('undo'), 'menu offers undo');
        check(indicator._doneNotification === null, 'completion from the open menu shows no notification');
        check(!targets().has(`task:${ci}:done`), 'the done task left the list');
        // Definitive service failures leave the completion and its offer intact.
        const beforePauseFailure = indicator._entry;
        this._rejectPause = true;
        targets().get('pause').emit('clicked', 1);
        await this._wait(() => this._callsNamed('TogglePause').length === 3, 'failed pause reaches fixture');
        await this._wait(() => indicator._entry !== beforePauseFailure, 'failed pause reply reconciles');
        this._rejectPause = false;
        check(indicator._undo?.id === ci && targets().has('undo'), 'failed pause preserves undo');
        const beforeFailure = indicator._entry;
        this._rejectPromote = true;
        targets().get(`task:${landlord}:promote`).emit('clicked', 1);
        await this._wait(() => this._callsNamed('Promote').length === 1, 'failed promote reaches fixture');
        await this._wait(() => indicator._entry !== beforeFailure, 'failed promote reply reconciles');
        this._rejectPromote = false;
        check(indicator._undo?.id === ci && targets().has('undo'), 'failed promote preserves undo');
        await this._pointer(targets().get(`task:${landlord}:done`).get_parent().get_parent());
        await this._shot('menu-undo');
        targets().get('undo').emit('clicked', 1);
        await this._wait(() => this._callsNamed('UndoComplete').length === 2, 'undo reaches UndoComplete');
        await this._wait(() => !indicator._undo && targets().has(`task:${ci}:done`), 'undo restores the task');

        // Done on the card completes the current task; promoting drops the offer.
        targets().get('done').emit('clicked', 1);
        await this._wait(() => indicator._undo?.id === fix, 'card done offers undo');
        check(indicator._label.text === 'Write release notes 0.2.0', 'the head of Next was pulled into Now');
        targets().get(`task:${landlord}:promote`).emit('clicked', 1);
        await this._wait(() => this._callsNamed('Promote').length === 2, 'promote reaches Promote');
        await this._wait(() => indicator._label.text === 'Wait for landlord reply' && !indicator._undo,
            'promotion makes the task current and withdraws undo');
        check(indicator._dot.has_style_class_name('qf-dot-personal'), 'dot follows the current tag');
        check(indicator._state.now.length === 1 && indicator._state.next[0].id === notes,
            'the task it replaced leads Next instead of staying in Now');

        // The gear opens Settings and closes the menu.
        targets().get('open:settings').emit('clicked', 1);
        await this._wait(() => this._callsNamed('Show').some(c => c.args[0] === 'settings'), 'gear shows settings');
        await this._wait(() => !indicator.menu.isOpen, 'view buttons close the menu');
        check(indicator._cardClock === null, 'a closed menu keeps no clock to tick for');

        // From a shortcut, with the menu closed, undo is offered in a notification.
        indicator.completeCurrent();
        await this._wait(() => indicator._undo && indicator._doneNotification, 'shortcut completion notifies');
    }

    /**
     * Titles are shown whole and on one line: the panel button and the menu
     * grow sideways to fit them, and only a title the screen has no room for
     * is cut short — without moving the clock, squeezing anything else on the
     * panel, or taking the menu off the monitor.
     */
    async _widths(indicator) {
        indicator._dropUndo();
        indicator.menu.close();
        const q = this._queue = new Queue();
        const monitor = Main.layoutManager.primaryMonitor;
        const {_leftBox: left, _centerBox: center, _rightBox: right} = Main.panel;
        const activities = Main.panel.statusArea.activities.container;
        check(left.get_child_at_index(0) === activities && left.get_child_at_index(1) === indicator.container,
            'the indicator sits beside Activities');
        const natural = actor => actor.get_preferred_width(-1)[1];
        const right_of = actor => actor.get_transformed_position()[0] + actor.get_transformed_size()[0];
        const middle = actor => actor.get_transformed_position()[0] + actor.get_transformed_size()[0] / 2;
        const settled = () => new Promise(resolve => {
            const id = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 250, () => {
                this._sources.delete(id);
                resolve();
                return GLib.SOURCE_REMOVE;
            });
            this._sources.add(id);
        });
        const menu = indicator.menu.actor;
        const titles = () => labels(menu).filter(l => ['qf-card-title', 'qf-side-title', 'qf-undo-text']
            .some(c => l.has_style_class_name(c)));
        const cut = l => l.clutter_text.get_layout().is_ellipsized();
        const lines = l => l.clutter_text.get_layout().get_line_count();
        // Whatever the title does, the rest of the panel stays as it was.
        const panelIntact = when => {
            check(Math.abs(middle(center) - (monitor.x + monitor.width / 2)) <= 1, `${when}: the clock stays in the middle`);
            check(activities.width === natural(activities), `${when}: Activities keeps its width`);
            check(right.width === natural(right), `${when}: the right of the panel keeps its width`);
            check(right_of(left) <= center.get_transformed_position()[0] &&
                right_of(center) <= right.get_transformed_position()[0], `${when}: the panel's boxes do not overlap`);
        };
        const menuOnScreen = when => {
            const [x] = menu.get_transformed_position();
            check(x >= monitor.x && right_of(menu) <= monitor.x + monitor.width,
                `${when}: the menu is on the monitor (${x}..${right_of(menu)} of ${monitor.width})`);
        };
        const show = async (current, side) => {
            q.tasks = [];
            q.add(current, 'now', 'work');
            const ids = side.map(title => q.add(title, 'side', 'personal'));
            this._changed();
            await this._wait(() => indicator._label.text === current, 'panel shows the new title');
            await settled();
            return ids;
        };

        // Short titles: the menu is the size the design drew.
        await show('Fix login redirect loop', ['CI run for main']);
        indicator.menu.open();
        await settled();
        const drawn = menu.width;
        check(titles().length === 2 && titles().every(l => !cut(l) && lines(l) === 1), 'short titles are whole');
        panelIntact('short');
        menuOnScreen('short');

        // Long ones, arriving while the menu is open: everything grows to fit.
        const long = 'Reconcile the payments export with the ledger totals';
        const longSide = 'Wait for the landlord to confirm the lease dates';
        const [sideId] = await show(long, [longSide, 'CI run for main']);
        check(!cut(indicator._label) && indicator._label.width >= natural(indicator._label.clutter_text),
            'the panel shows a long title whole');
        check(titles().length === 3, 'the card and both Side cards carry a title');
        for (const l of titles()) {
            check(!cut(l) && lines(l) === 1, `"${l.text}" is whole and on one line`);
            check(l.width >= natural(l.clutter_text), `"${l.text}" has the width it asks for`);
        }
        check(menu.width > drawn, `the menu grew to fit (${drawn} -> ${menu.width})`);
        panelIntact('long');
        menuOnScreen('long');
        const sideCard = indicator._focusTargets.get(`task:${sideId}:done`).get_parent().get_parent();
        await this._pointer(sideCard);
        await this._wait(() => sideCard.hover, 'pointer hovers the long side card');
        await this._shot('menu-long');

        // A title no screen has room for is the one case that is cut short:
        // the panel stops it at the clock and the menu stays on the monitor.
        const absurd = Array.from({length: 32}, (_v, i) => `word${i} and`).join(' ').slice(0, 256);
        await show(absurd, [absurd]);
        check(cut(indicator._label), 'a title that would reach the clock is cut short');
        check(left.width < natural(left), 'by the panel, which gives the left box no more than that');
        check(titles().every(l => cut(l) && lines(l) === 1), 'a title wider than the screen is cut short, never wrapped');
        panelIntact('absurd');
        menuOnScreen('absurd');
        check(menu.width <= monitor.width, 'the menu is no wider than the monitor');
        await this._shot('menu-absurd');

        // While it stays open the menu only grows, so nothing slides out from
        // under the pointer; closing it lets go of the width.
        const widest = menu.width;
        await show('Fix login redirect loop', [longSide, 'CI run for main']);
        check(menu.width === widest, `the open menu kept its width (${widest} -> ${menu.width})`);
        panelIntact('short again');
        indicator.menu.close();
        indicator.menu.open();
        await settled();
        const reopened = menu.width;
        check(reopened < widest && reopened > drawn, `reopened, it fits what it holds (${drawn} < ${reopened} < ${widest})`);

        // Done on the card with the longest title: the column it widened
        // could narrow, and the offer to undo names that title in a column
        // too narrow for it. Neither may move the button beside the pointer.
        const targets = () => indicator._focusTargets;
        const [longId, ciId] = q.tasks.filter(t => t.bucket === 'side').map(t => t.id);
        const left_of = actor => actor.get_transformed_position()[0];
        const before = left_of(targets().get(`task:${ciId}:done`));
        targets().get(`task:${longId}:done`).emit('clicked', 1);
        await this._wait(() => targets().has('undo'), 'menu offers undo for the long task');
        await settled();
        check(menu.width === reopened, `done left the menu its width (${reopened} -> ${menu.width})`);
        check(left_of(targets().get(`task:${ciId}:done`)) === before, 'the next Done button stayed where it was');
        const offer = titles().find(l => l.has_style_class_name('qf-undo-text'));
        check(offer.text === `Done · ${longSide}` && cut(offer) && lines(offer) === 1,
            'the undo offer fits the column it is in');
        menuOnScreen('undo');
        await this._shot('menu-steady-undo');
        indicator.menu.close();
        indicator.menu.open();
        await settled();
        check(menu.width === drawn, `reopened, the menu is back to the size drawn (${menu.width} vs ${drawn})`);
        indicator._dropUndo();
        indicator.menu.close();
        await show(long, ['CI run for main']);
        q.add('Short next task', 'next');
        this._changed();
        indicator.menu.open();
        await settled();
        const bounds = key => {
            const actor = targets().get(key);
            return [left_of(actor), right_of(actor)];
        };
        const doneBounds = bounds('done');
        const pauseBounds = bounds('pause');
        targets().get('done').emit('clicked', 1);
        await this._wait(() => indicator._label.text === 'Short next task' && targets().has('undo'),
            'completing the long current task pulls the short next task');
        await settled();
        for (const [key, beforeBounds] of [['done', doneBounds], ['pause', pauseBounds]]) {
            const afterBounds = bounds(key);
            check(afterBounds.every((edge, i) => Math.abs(edge - beforeBounds[i]) <= 1),
                `${key} stays put after completing a long current task (${beforeBounds} -> ${afterBounds})`);
        }
        menuOnScreen('short replacement');
        indicator._dropUndo();
        indicator.menu.close();
    }

    disable() {
        for (const id of this._sources) GLib.source_remove(id);
        this._sources.clear();
        if (this._ownerId !== null && this._ownerId !== undefined) Gio.bus_unown_name(this._ownerId);
        this._fixture?.unexport();
    }
}
