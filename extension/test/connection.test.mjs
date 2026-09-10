// Drive the shipped connection module without a bus, wall clock or GNOME actors.
import assert from 'node:assert/strict';
import {test} from 'node:test';
import {QueueConnection} from '../queue-focus@queuefocus.org/connection.js';

class Runtime {
    owner = ':1.10';
    calls = [];
    timers = new Map();
    nextTimer = 1;
    start(events) { this.events = events; }
    call(method, args, done) { this.calls.push({method, args, done}); }
    schedule(ms, callback) {
        const id = this.nextTimer++;
        this.timers.set(id, {ms, callback});
        return id;
    }
    cancel(id) { this.timers.delete(id); }
    isUnavailable(error) { return error.unavailable === true; }
    errorMessage(error) { return error.message; }
    destroy() { this.closed = true; }
    changeOwner(owner) {
        this.owner = owner;
        this.events.ownerChanged(owner);
    }
    signal(name, value, sender = this.owner) {
        this.events.signal(sender, name, typeof value === 'object' ? JSON.stringify(value) : value);
    }
    latest(method) {
        return this.calls.filter(call => call.method === method).at(-1);
    }
    reply(method, value, error = null) {
        this.latest(method).done(error ? null : [JSON.stringify(value)], error);
    }
    tick() {
        assert.equal(this.timers.size, 1);
        const [id, {callback, ms}] = [...this.timers][0];
        this.timers.delete(id);
        callback();
        return ms;
    }
}
const snapshot = title => ({current: {id: 1, title}, now: [], side: [], next: [], later: []});
function setup(owner = ':1.10') {
    const runtime = new Runtime();
    runtime.owner = owner;
    const seen = {state: [], settings: [], flash: [], warning: []};
    const connection = new QueueConnection(runtime, Object.fromEntries(
        Object.keys(seen).map(key => [key, value => seen[key].push(value)])));
    runtime.events.ready(null);
    return {runtime, seen, connection};
}
function healthy(runtime) {
    runtime.reply('GetSettings', {show_timer: false});
    runtime.reply('GetState', snapshot('current'));
}

test('initial snapshots and live signals reach observers', () => {
    const {runtime, seen} = setup();
    healthy(runtime);
    runtime.signal('Changed', snapshot('updated'));
    runtime.signal('SettingsChanged', {show_timer: true});
    runtime.signal('Flash', {title: 'updated'});
    runtime.signal('DurabilityWarning', 'saved, but not crash-safe');
    assert.deepEqual(seen.state.map(s => s.current.title), ['current', 'updated']);
    assert.deepEqual(seen.settings, [{show_timer: false}, {show_timer: true}]);
    assert.deepEqual(seen.flash, [{title: 'updated'}]);
    assert.deepEqual(seen.warning, ['saved, but not crash-safe']);
});

test('crashes retry with capped backoff and successful recovery resets it', () => {
    const {runtime, seen} = setup();
    healthy(runtime);
    runtime.changeOwner(null);
    assert.equal(seen.state.at(-1), null);
    for (const expected of [1500, 3000, 6000, 12000, 24000, 48000, 60000, 60000]) {
        assert.equal(runtime.tick(), expected);
        runtime.reply('GetState', null, new Error('startup failed'));
    }
    runtime.changeOwner(':1.11');
    assert.equal(runtime.timers.size, 0);
    healthy(runtime);
    runtime.changeOwner(null);
    assert.equal(runtime.tick(), 1500);
});

test('announced stopping cancels retries and pending read failures cannot restart it', () => {
    const {runtime, connection} = setup();
    const read = runtime.latest('GetState');
    runtime.reply('GetSettings', null, new Error('temporary'));
    runtime.signal('Stopping');
    runtime.changeOwner(null);
    read.done(null, new Error('stopped'));
    assert.equal(runtime.timers.size, 0);
    const before = runtime.calls.length;
    connection.refresh();
    assert.equal(runtime.calls.length, before + 2, 'explicit start wakes the app');
});

test('uninstalled app stays idle even if settings fails after state', () => {
    const {runtime, connection} = setup(null);
    const settings = runtime.latest('GetSettings');
    runtime.reply('GetState', null, Object.assign(new Error('not installed'), {unavailable: true}));
    settings.done(null, new Error('not installed'));
    assert.equal(runtime.timers.size, 0);
    const before = runtime.calls.length;
    connection.request('Show', ['queue']);
    assert.equal(runtime.calls.length, before + 1);
    assert.equal(runtime.latest('Show').args[0], 'queue');
});

test('old owner replies and signals cannot replace the new snapshot', () => {
    const {runtime, seen} = setup();
    const oldState = runtime.latest('GetState');
    const oldSettings = runtime.latest('GetSettings');
    runtime.changeOwner(':1.11');
    healthy(runtime);
    oldState.done([JSON.stringify(snapshot('obsolete'))], null);
    oldSettings.done(['{"show_timer":true}'], null);
    runtime.signal('Changed', snapshot('obsolete'), ':1.10');
    runtime.signal('Stopping', undefined, ':1.10');
    assert.equal(seen.state.at(-1).current.title, 'current');
    assert.equal(seen.settings.at(-1).show_timer, false);
    runtime.changeOwner(null);
    assert.equal(runtime.timers.size, 1, 'old stopping signal was ignored');
});

test('live signals supersede reads already in flight', () => {
    const {runtime, seen} = setup();
    runtime.signal('Changed', snapshot('newer'));
    runtime.signal('SettingsChanged', {show_timer: true});
    healthy(runtime);
    assert.deepEqual(seen.state.map(s => s.current.title), ['newer']);
    assert.deepEqual(seen.settings, [{show_timer: true}]);
});

test('interrupted completion settles once, without replaying or offering stale undo', () => {
    const {runtime, connection} = setup();
    healthy(runtime);
    const outcomes = [];
    connection.request('CompleteCurrent', [], (...args) => outcomes.push(args));
    const completion = runtime.latest('CompleteCurrent');
    runtime.changeOwner(null);
    assert.equal(outcomes.length, 1);
    assert.match(outcomes[0][1].message, /before the result could be confirmed/);
    runtime.tick();
    runtime.changeOwner(':1.11');
    healthy(runtime);
    completion.done([1, 'previous task'], null);
    assert.equal(outcomes.length, 1);
    assert.equal(runtime.calls.filter(c => c.method === 'CompleteCurrent').length, 1);
});

test('an action that activates an idle app may complete on its first owner', () => {
    const {runtime, connection} = setup(null);
    const outcomes = [];
    connection.request('Add', ['draft', ''], (...args) => outcomes.push(args));
    runtime.changeOwner(':1.11');
    runtime.latest('Add').done([7], null);
    assert.deepEqual(outcomes, [[[7], null]]);
    assert.equal(runtime.calls.filter(c => c.method === 'Add').length, 1);
});

test('action failures are reported once and reconcile with reads', () => {
    const {runtime, connection} = setup();
    const outcomes = [];
    connection.request('Promote', [7], (...args) => outcomes.push(args));
    const before = runtime.calls.length;
    runtime.latest('Promote').done(null, new Error('reply lost'));
    assert.match(outcomes[0][1].message, /reply lost/);
    assert.equal(runtime.calls.length, before + 2);
    assert.equal(runtime.calls.filter(c => c.method === 'Promote').length, 1);
});

test('settings failures are retried even when task state is healthy', () => {
    const {runtime, seen} = setup();
    runtime.reply('GetSettings', null, new Error('temporary'));
    runtime.reply('GetState', snapshot('current'));
    assert.equal(runtime.timers.size, 1);
    runtime.tick();
    healthy(runtime);
    assert.deepEqual(seen.settings, [{show_timer: false}]);
    assert.equal(runtime.timers.size, 0);
});

test('malformed payloads do not escape into drawing; invalid state is retried', () => {
    const {runtime, seen} = setup();
    runtime.signal('Changed', '{broken');
    runtime.signal('SettingsChanged', '[]');
    runtime.signal('Flash', '{broken');
    assert.deepEqual(seen.state, [null]);
    assert.deepEqual(seen.settings, []);
    assert.deepEqual(seen.flash, []);
    assert.equal(runtime.timers.size, 1);
});

test('disable silences callbacks, disconnects runtime, and cancels recovery once', () => {
    const {runtime, connection, seen} = setup();
    let called = false;
    connection.request('CompleteCurrent', [], () => { called = true; });
    runtime.reply('GetState', null, new Error('temporary'));
    connection.destroy();
    connection.destroy();
    const before = structuredClone(seen);
    const count = runtime.calls.length;
    runtime.latest('CompleteCurrent').done([1, 'done'], null);
    runtime.events.ready(null);
    runtime.changeOwner(':1.12');
    runtime.signal('Changed', snapshot('late'));
    runtime.reply('GetSettings', {show_timer: false});
    connection.refresh();
    connection.request('Show', ['queue']);
    assert.equal(called, false);
    assert.equal(runtime.closed, true);
    assert.equal(runtime.timers.size, 0);
    assert.equal(runtime.calls.length, count);
    assert.deepEqual(seen, before);
});

for (const settingsFirst of [true, false]) {
    test(`persistent settings failures back off (${settingsFirst ? 'settings' : 'state'} replies first)`, () => {
        const {runtime} = setup();
        for (const expected of [1500, 3000, 6000, 12000, 24000, 48000, 60000, 60000]) {
            const settings = () => runtime.reply('GetSettings', null, new Error('unavailable settings'));
            const state = () => runtime.reply('GetState', snapshot('current'));
            if (settingsFirst) { settings(); state(); }
            else { state(); settings(); }
            // Regular queue changes must not disguise the settings failure.
            runtime.signal('Changed', snapshot('updated'));
            assert.equal(runtime.tick(), expected);
        }
        healthy(runtime);
        runtime.changeOwner(null);
        assert.equal(runtime.tick(), 1500, 'complete recovery resets the next wait');
    });
}

test('valid live settings recover a failed read and cancel its scheduled retry', () => {
    const {runtime} = setup();
    runtime.reply('GetSettings', null, new Error('temporary'));
    runtime.reply('GetState', snapshot('current'));
    assert.equal(runtime.timers.size, 1);
    runtime.signal('SettingsChanged', {show_timer: true});
    assert.equal(runtime.timers.size, 0);
});
