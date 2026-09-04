// Flash overlay tests. Run with `make test-extension`.
//
// The overlay draws through GNOME Shell, which cannot run here, so the toolkit
// is stubbed (see gi-stubs.mjs) and these tests check what the overlay *builds*:
// the layers each style needs, where they land on the screen, and that every
// path takes the overlay back down again.
import assert from 'node:assert/strict';
import {register} from 'node:module';

register('./loader.mjs', import.meta.url);

const stubs = await import('./gi-stubs.mjs');
const {layoutManager, uiGroup, GLib, settings} = stubs;
// Imported after the resolver is registered, so its `gi://` imports resolve.
const {FlashOverlay} = await import('../queue-focus@queuefocus.org/flash.js');

const STYLES = ['wash', 'wash2', 'edges', 'edgesSoft', 'topbar', 'topbarBeam'];
const flash = (style, extra = {}) => ({
    style,
    intensity: 'normal',
    palette: 'blue',
    title: 'ship v0.1',
    timer: '23m',
    ...extra,
});
const box = actor => ({x: actor.x, y: actor.y, w: actor.width, h: actor.height});
const overlay = () => uiGroup.children.at(-1);
const card = () => overlay().children.at(-1).children[0];
const strips = layer => layer.children.filter(c => String(c.style).includes('gradient'));

let failed = 0;
function test(name, fn) {
    stubs.reset();
    try {
        fn();
        console.log(`  ok  ${name}`);
    } catch (e) {
        failed++;
        console.log(`FAIL  ${name}\n      ${e.message.split('\n').join('\n      ')}`);
    }
}

for (const style of STYLES) {
    test(`${style}: covers the monitor and puts the card on top`, () => {
        const monitor = layoutManager.primaryMonitor;
        new FlashOverlay().show(flash(style));

        assert.equal(overlay().reactive, false, 'a flash never takes the pointer');
        assert.deepEqual(box(overlay()), {x: 0, y: 0, w: monitor.width, h: monitor.height});
        assert.ok(overlay().children.length >= 1, 'draws at least one layer');
        assert.equal(card().style_class, 'qf-flash-card');
        assert.deepEqual(card().children.map(c => c.style_class),
            ['qf-flash-now', 'qf-flash-title', 'qf-flash-timer']);
        assert.equal(card().children[0].text, 'NOW');
        assert.equal(card().children[1].text, 'ship v0.1');
        assert.equal(card().children[2].text, '23m');
    });
}

test('a task whose clock has not started drops the timer line', () => {
    new FlashOverlay().show(flash('wash', {timer: ''}));
    assert.deepEqual(card().children.map(c => c.style_class),
        ['qf-flash-now', 'qf-flash-title']);
});

test('an untrusted title is bounded before Clutter lays it out', () => {
    new FlashOverlay().show(flash('wash', {title: `  first\n\t${'x'.repeat(10000)}`}));
    const label = card().children[1];
    assert.ok([...label.text].length <= 256, 'the rendered text has a hard character cap');
    assert.ok(label.text.startsWith('first x'), 'whitespace is folded into one paragraph');
    assert.ok(label.text.endsWith('…'), 'truncation is visible');
    assert.equal(label.clutter_text.max_length, 256, 'Clutter enforces the cap too');
    assert.equal(label.clutter_text.ellipsize, 'end', 'the constrained label ellipsizes');
});

test('bounding does not scan through an unbounded whitespace tail', () => {
    new FlashOverlay().show(flash('wash', {title: `first${' '.repeat(10000)}last`}));
    const text = card().children[1].text;
    assert.ok([...text].length <= 256);
    assert.equal(text, 'first…');
});

test('the personal palette colours the card, not the title', () => {
    new FlashOverlay().show(flash('wash', {palette: 'orange'}));
    assert.match(card().children[0].style, /#ffa348/, 'NOW takes the accent');
    assert.match(card().children[2].style, /#ffa348/, 'so does the clock');
    assert.equal(card().children[1].style, undefined, 'the title stays white');
    assert.match(String(overlay().children[0].style), /230, 97, 0/, 'the wash goes orange');
});

for (const [style, inset] of [['edges', 8], ['edgesSoft', 0]]) {
    test(`${style}: the glow runs the whole of each side, corners included`, () => {
        const {width, height} = layoutManager.primaryMonitor;
        new FlashOverlay().show(flash(style));
        const found = strips(overlay().children[0]).map(box);

        assert.equal(found.length, 4, 'one strip per side');
        for (const s of found) {
            assert.ok(s.w > 0 && s.h > 0, `positive size ${JSON.stringify(s)}`);
            assert.ok(s.x >= inset && s.y >= inset, `clear of the ring ${JSON.stringify(s)}`);
            assert.ok(s.x + s.w <= width - inset && s.y + s.h <= height - inset,
                `inside the screen ${JSON.stringify(s)}`);
        }
        // Every corner is covered twice. Butting the strips together instead
        // would leave a seam there, where one strip's clear end meets the
        // next one's solid start.
        const covering = (px, py) =>
            found.filter(s => px >= s.x && px < s.x + s.w && py >= s.y && py < s.y + s.h).length;
        for (const [px, py] of [
            [inset, inset],
            [width - inset - 1, inset],
            [inset, height - inset - 1],
            [width - inset - 1, height - inset - 1],
        ]) {
            assert.equal(covering(px, py), 2, `corner ${px},${py} is covered by both its sides`);
        }
        // And the middle of the screen is covered by none of them.
        assert.equal(covering(Math.round(width / 2), Math.round(height / 2)), 0);
    });
}

test('the glow fades to a clear accent, never to black', () => {
    new FlashOverlay().show(flash('edgesSoft'));
    for (const strip of strips(overlay().children[0])) {
        assert.ok(!strip.style.includes('transparent'),
            'fading to `transparent` would dirty the middle of the fade');
        assert.match(strip.style, /rgba\(53, 132, 228, 0\)/);
    }
});

test('a screen too small for the glow still gets one', () => {
    layoutManager.primaryMonitor = {x: 0, y: 0, width: 120, height: 90};
    new FlashOverlay().show(flash('edgesSoft', {intensity: 'strong'}));
    for (const strip of strips(overlay().children[0])) {
        const s = box(strip);
        assert.ok(s.w > 0 && s.h > 0, `positive size ${JSON.stringify(s)}`);
    }
});

test('the beam runs from the panel down to the middle of the screen', () => {
    const {width, height} = layoutManager.primaryMonitor;
    const panel = layoutManager.panelBox.height;
    new FlashOverlay().show(flash('topbarBeam'));

    assert.deepEqual(box(overlay().children[0]), {x: 0, y: 0, w: width, h: panel},
        'the bar covers the panel exactly');
    const beam = overlay().children[1];
    assert.deepEqual(box(beam),
        {x: Math.round((width - 3) / 2), y: panel, w: 3, h: height / 2 - panel});
    assert.deepEqual(beam.pivot, [0.5, 0], 'so it grows downwards');
});

test('the plain top bar style draws no beam', () => {
    new FlashOverlay().show(flash('topbar'));
    assert.equal(overlay().children.length, 2, 'the bar and the card, nothing else');
});

test('intensity changes how loud a flash is', () => {
    const wash = intensity => {
        stubs.reset();
        new FlashOverlay().show(flash('wash', {intensity}));
        return overlay().children[0].style;
    };
    assert.match(wash('subtle'), /0\.14/);
    assert.match(wash('normal'), /0\.26/);
    assert.match(wash('strong'), /0\.42/);
});

test('a flash replaces one that is still running', () => {
    const overlays = new FlashOverlay();
    overlays.show(flash('wash'));
    const first = overlay();
    overlays.show(flash('edges'));
    assert.ok(first.destroyed, 'the one it interrupted is gone');
    assert.ok(!overlay().destroyed);
});

test('clearing an overlay twice, or one that never showed, is harmless', () => {
    const overlays = new FlashOverlay();
    overlays.clear();
    overlays.show(flash('wash'));
    overlays.clear();
    overlays.clear();
    overlays.destroy();
});

/// With animations off every ease lands at once, so the flash has to be held
/// on a timer instead — otherwise the reminder is never seen at all.
test('with animations off the flash is held at its peak, then taken down', () => {
    settings.enable_animations = false;
    new FlashOverlay().show(flash('topbarBeam'));

    for (const layer of overlay().children)
        assert.equal(layer.opacity, 255, 'every layer is at full opacity');
    assert.equal(overlay().children[1].scale_y, 1, 'the beam is drawn, not scaled away');
    assert.equal(overlay().children.at(-1).translation_y, 0, 'the card has landed');

    assert.ok(GLib.pendingHold, 'a hold is pending');
    // A repeating source that forgets to say so would fire for ever.
    assert.equal(GLib.pendingHold(), GLib.SOURCE_REMOVE, 'the hold runs once');
    assert.ok(overlay().destroyed, 'and it takes the flash down');
});

test('a style or palette this version does not know still shows the card', () => {
    new FlashOverlay().show(flash('semaphore', {intensity: 'blinding', palette: 'puce'}));
    assert.equal(overlay().children.length, 1, 'the card alone');
    assert.equal(card().style_class, 'qf-flash-card');
});

console.log(failed ? `\n${failed} failed` : '\nextension flash tests passed');
process.exit(failed ? 1 : 0);
