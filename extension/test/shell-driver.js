// Test-only extension loaded alongside the shipped extension in a disposable
// GNOME Shell. No production handlers or actors are replaced.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const UUID = 'queue-focus@queuefocus.org';
const BUS_NAME = 'org.queuefocus.QueueFocus';
const xml = `<node><interface name="org.queuefocus.QueueFocus1">
<method name="GetState"><arg type="s" direction="out"/></method>
<method name="GetSettings"><arg type="s" direction="out"/></method>
<method name="Add"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="t" direction="out"/></method>
<signal name="Changed"><arg type="s"/></signal>
<signal name="Stopping"/>
</interface></node>`;
const empty = () => JSON.stringify({current: null, now: [], side: [], next: [], later: []});
const check = (value, message) => { if (!value) throw new Error(message); };

export default class ShellTest extends Extension {
    enable() {
        this._sources = new Set();
        this._requests = [];
        this._bus = Gio.DBus.session;
        this._fixture = Gio.DBusExportedObject.wrapJSObject(xml, {
            GetState: empty,
            GetSettings: () => JSON.stringify({show_timer: false}),
            AddAsync: (args, invocation) => this._requests.push({args, invocation}),
        });
        this._fixture.export(this._bus, '/org/queuefocus/QueueFocus');
        this._run().then(() => this._result({ok: true})).catch(error => {
            logError(error);
            this._result({ok: false, error: error.message, stack: error.stack});
        });
    }

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

        await this._own();
        await this._wait(() => indicator._state !== null, 'indicator reconnects');
        this._requests[0].invocation.return_value(new GLib.Variant('(t)', [1]));
        this._fixture.emit_signal('Changed', new GLib.Variant('(s)', [empty()]));
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
    }

    disable() {
        for (const id of this._sources) GLib.source_remove(id);
        this._sources.clear();
        if (this._ownerId !== null && this._ownerId !== undefined) Gio.bus_unown_name(this._ownerId);
        this._fixture?.unexport();
    }
}
