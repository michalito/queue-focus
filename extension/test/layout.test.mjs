// Width limits for the top bar. Run with `make test-extension`.
//
// layout.js is plain arithmetic, so it is checked against the panel's own
// allocation rule rather than a stubbed shell: `allocate` below is what
// GNOME Shell's panel does with the three boxes (js/ui/panel.js, 48 to 50).
import assert from 'node:assert/strict';

const {menuMaxWidth, panelTitleRoom} = await import('../queue-focus@queuefocus.org/layout.js');

let failed = 0;
function test(name, fn) {
    try {
        fn();
        console.log(`  ok  ${name}`);
    } catch (e) {
        failed++;
        console.log(`FAIL  ${name}\n      ${e.message.split('\n').join('\n      ')}`);
    }
}

/** Where the panel puts its boxes, left to right, for what each asks for. */
function allocate({panelWidth, centerOffset, startWidth, centerWidth, endWidth}) {
    const sideWidth = Math.max(0, (panelWidth - centerWidth + centerOffset) / 2);
    const start = {x1: 0, x2: Math.min(Math.floor(sideWidth), startWidth)};
    const center = {x1: Math.ceil(sideWidth), x2: Math.ceil(sideWidth) + centerWidth};
    const end = {x1: Math.max(panelWidth - Math.min(Math.floor(sideWidth), endWidth), 0), x2: panelWidth};
    return {start, center, end};
}

/** The panel once the title has taken all the room it was given. */
function filled(panel, others, gap = 12, scale = 1) {
    const room = panelTitleRoom({...panel, centerWidth: others, titleWidth: 0, gap, scale});
    return {room, boxes: allocate({...panel, centerWidth: others + room * scale})};
}

const PANELS = [
    ['a bare panel', {panelWidth: 1920, centerOffset: 0, startWidth: 80, endWidth: 240}],
    ['a fuller left than right', {panelWidth: 1920, centerOffset: 0, startWidth: 420, endWidth: 90}],
    ['a dock on the left', {panelWidth: 1920, centerOffset: 72, startWidth: 80, endWidth: 240}],
    ['a dock on the right', {panelWidth: 1920, centerOffset: -72, startWidth: 240, endWidth: 80}],
    ['an odd width', {panelWidth: 1367, centerOffset: 0, startWidth: 81, endWidth: 233}],
];

for (const [name, panel] of PANELS) {
    test(`${name}: a title filling its room leaves both sides whole, with air between`, () => {
        const {boxes} = filled(panel, 190);
        assert.equal(boxes.start.x2 - boxes.start.x1, panel.startWidth, 'start box keeps its width');
        assert.equal(boxes.end.x2 - boxes.end.x1, panel.endWidth, 'end box keeps its width');
        assert.ok(boxes.center.x1 - boxes.start.x2 >= 12, `gap at the start: ${boxes.center.x1 - boxes.start.x2}`);
        assert.ok(boxes.end.x1 - boxes.center.x2 >= 12, `gap at the end: ${boxes.end.x1 - boxes.center.x2}`);
    });

    test(`${name}: the room is all there is — two pixels more and a side gives way`, () => {
        const {room} = filled(panel, 190);
        const boxes = allocate({...panel, centerWidth: 190 + room + 2});
        const gaps = [boxes.center.x1 - boxes.start.x2, boxes.end.x1 - boxes.center.x2];
        assert.ok(Math.min(...gaps) < 12, `gaps ${gaps}`);
    });
}

test('what else is in the centre box comes out of the room, the title itself does not', () => {
    const panel = {panelWidth: 1920, centerOffset: 0, startWidth: 80, endWidth: 240, gap: 12, scale: 1};
    const alone = panelTitleRoom({...panel, centerWidth: 0, titleWidth: 0});
    assert.equal(alone, 1920 - 2 * (240 + 12));
    assert.equal(panelTitleRoom({...panel, centerWidth: 190, titleWidth: 0}), alone - 190);
    // However wide the title is being shown right now, the answer is the same.
    assert.equal(panelTitleRoom({...panel, centerWidth: 190 + 320, titleWidth: 320}), alone - 190);
    assert.equal(panelTitleRoom({...panel, centerWidth: 190 + 900, titleWidth: 900}), alone - 190);
});

test('the room is in logical pixels, whatever the scale', () => {
    const panel = {centerOffset: 0, gap: 12};
    const once = panelTitleRoom({...panel, panelWidth: 1920, startWidth: 80, endWidth: 240,
        centerWidth: 190, titleWidth: 0, scale: 1});
    const gapless = panelTitleRoom({...panel, panelWidth: 3840, startWidth: 160, endWidth: 480,
        centerWidth: 380, titleWidth: 0, gap: 24, scale: 2});
    assert.equal(gapless, once);
});

test('a crowded panel still shows a title worth reading', () => {
    const crowded = {panelWidth: 800, centerOffset: 0, startWidth: 300, endWidth: 320, centerWidth: 150,
        titleWidth: 0, gap: 12, scale: 1};
    assert.equal(panelTitleRoom(crowded), 160);
});

test('the menu fits the work area, less the edge the shell keeps on both sides', () => {
    assert.equal(menuMaxWidth({workAreaWidth: 1920, edge: 6, margins: 0, scale: 1}), 1908);
    assert.equal(menuMaxWidth({workAreaWidth: 1848, edge: 6, margins: 0, scale: 1}), 1836);
    assert.equal(menuMaxWidth({workAreaWidth: 3840, edge: 12, margins: 0, scale: 2}), 1908);
    // Margins are the menu's own, counted once: they are already both sides.
    assert.equal(menuMaxWidth({workAreaWidth: 1920, edge: 6, margins: 20, scale: 1}), 1888);
    assert.equal(menuMaxWidth({workAreaWidth: 10, edge: 12, margins: 0, scale: 1}), 0);
});

if (failed) {
    console.log(`\n${failed} failed`);
    process.exit(1);
}
console.log('\nall layout tests passed');
