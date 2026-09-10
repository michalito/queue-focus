// Run only on a private bus: dbus-run-session -- gjs -m extension/test/dbus.test.js
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import System from 'system';
import {connectQueue} from '../queue-focus@queuefocus.org/dbus.js';

const loop = new GLib.MainLoop(null, false);
const bus = Gio.DBus.session;
const name = 'org.queuefocus.QueueFocus';
const path = '/org/queuefocus/QueueFocus';
const xml = `<node><interface name="org.queuefocus.QueueFocus1">
<method name="GetState"><arg type="s" direction="out"/></method>
<method name="GetSettings"><arg type="s" direction="out"/></method>
<method name="CompleteCurrent"><arg type="t" direction="out"/><arg type="s" direction="out"/></method>
<method name="Promote"><arg type="t" direction="in"/></method>
<signal name="Changed"><arg type="s"/></signal>
<signal name="Stopping"/>
</interface></node>`;
let completion;
let completionCalls = 0;
let title = 'initial';
const snapshot = () => JSON.stringify({current: {id: 1, title}, now: [], side: [], next: [], later: []});
const exported = Gio.DBusExportedObject.wrapJSObject(xml, {
    GetState() { return snapshot(); },
    GetSettings() { return JSON.stringify({show_timer: false}); },
    PromoteAsync(_args, invocation) {
        invocation.return_dbus_error('org.freedesktop.DBus.Error.NoReply', 'Reply lost');
    },
    CompleteCurrentAsync(_args, invocation) {
        ++completionCalls;
        completion = invocation;
    },
});
exported.export(bus, path);
let ownerId;
let connection;
function check(value, message) { if (!value) throw new Error(message); }
function waitFor(predicate) {
    return new Promise((resolve, reject) => {
        const deadline = GLib.get_monotonic_time() + 5_000_000;
        GLib.timeout_add(GLib.PRIORITY_DEFAULT, 10, () => {
            if (predicate()) {
                resolve();
                return GLib.SOURCE_REMOVE;
            }
            if (GLib.get_monotonic_time() >= deadline) {
                reject(new Error('timed out waiting for D-Bus'));
                return GLib.SOURCE_REMOVE;
            }
            return GLib.SOURCE_CONTINUE;
        });
    });
}
function own() {
    return new Promise(resolve => {
        ownerId = Gio.bus_own_name_on_connection(bus, name, Gio.BusNameOwnerFlags.NONE, resolve, null);
    });
}
let failed = false;
async function run() {
    await own();
    const states = [];
    const settings = [];
    connection = connectQueue({
        state: value => states.push(value),
        settings: value => settings.push(value),
        flash: () => {},
        warning: () => {},
    });
    await waitFor(() => states.at(-1)?.current?.title === 'initial' && settings.length);
    check(settings.at(-1).show_timer === false, 'settings round trip');
    title = 'changed';
    exported.emit_signal('Changed', new GLib.Variant('(s)', [snapshot()]));
    await waitFor(() => states.at(-1)?.current?.title === 'changed');

    const outcomes = [];
    connection.request('CompleteCurrent', [], (...args) => outcomes.push(args));
    await waitFor(() => completion !== undefined);
    exported.emit_signal('Stopping', null);
    bus.flush_sync(null);
    Gio.bus_unown_name(ownerId);
    ownerId = null;
    await waitFor(() => outcomes.length === 1 && states.at(-1) === null);
    check(outcomes[0][1]?.message.includes('could be confirmed'), 'interruption reported');
    await own();
    await waitFor(() => states.at(-1)?.current?.title === 'changed');
    completion.return_value(new GLib.Variant('(ts)', [1, 'old completion']));
    bus.flush_sync(null);

    // A subsequent completed round trip lets the obsolete reply drain.
    connection.refresh();
    const count = states.length;
    await waitFor(() => states.length > count);
    check(outcomes.length === 1, 'obsolete completion suppressed');
    check(completionCalls === 1, 'completion never replayed');
    let failure;
    connection.request('Promote', [1], (_result, error) => { failure = error; });
    await waitFor(() => failure !== undefined);
    check(failure.message.includes('result could not be confirmed'), 'uncertain reply explained');
    connection.destroy();
    connection.destroy();
    print('D-Bus adapter integration passed: snapshots, signals, owner loss, stale reply, cleanup');
}
run().catch(error => {
    failed = true;
    logError(error);
}).finally(() => {
    connection?.destroy();
    if (ownerId !== null && ownerId !== undefined) Gio.bus_unown_name(ownerId);
    exported.unexport();
    loop.quit();
});
loop.run();
if (failed) System.exit(1);
