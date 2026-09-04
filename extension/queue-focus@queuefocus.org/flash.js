// Queue Focus — the screen flash the reminder asks for.
//
// The service decides when to flash, in which style, and what it says; this
// draws it. One overlay per flash, above every window, never in the pointer's
// way, and gone again in under two seconds.
//
// Three families, two variations each. `wash` tints the whole screen, `edges`
// lights its border, `topbar` colours the panel — and every one of them puts
// the same card in the middle of the screen.
import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Pango from 'gi://Pango';
import Shell from 'gi://Shell';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';

/** The two colour ways, as the service names them. */
const PALETTE = {
    blue: {accent: '53, 132, 228', text: '#62a0ea'},
    orange: {accent: '230, 97, 0', text: '#ffa348'},
};

/** How loud each intensity is. Lengths are logical pixels. */
const INTENSITY = {
    subtle: {wash: 0.14, edge: 4, glow: 24, edgeAlpha: 0.6, bar: 0.45, beam: 2, scale: 0.92},
    normal: {wash: 0.26, edge: 8, glow: 56, edgeAlpha: 0.85, bar: 0.7, beam: 3, scale: 1},
    strong: {wash: 0.42, edge: 14, glow: 100, edgeAlpha: 1, bar: 0.9, beam: 4, scale: 1.08},
};

// Opacity envelopes: where the run starts, then `[fraction of the run, opacity]`
// for each turn after that. Every one eases in and out. The layer's own colour
// already carries the intensity's alpha, so these run the full 0…1.
const ENVELOPE = {
    wash: {duration: 1600, from: 0, steps: [[0.25, 1], [0.70, 1], [1, 0]]},
    wash2: {duration: 1700, from: 0, steps: [[0.20, 1], [0.40, 0.45], [0.60, 1], [1, 0]]},
    edges: {duration: 1700, from: 0, steps: [[0.20, 1], [0.40, 0.5], [0.60, 1], [1, 0]]},
    breath: {duration: 1800, from: 0, steps: [[0.40, 1], [1, 0]]},
    bar: {duration: 1700, from: 0, steps: [[0.15, 1], [0.80, 1], [1, 0]]},
    beam: {duration: 1700, from: 1, steps: [[0.80, 1], [1, 0]]},
    card: {duration: 1700, from: 0, steps: [[0.20, 1], [0.80, 1], [1, 0]]},
};

const MODE = Clutter.AnimationMode.EASE_IN_OUT_QUAD;
/** How long a flash drawn without animations stays up, in ms. */
const STILL_MS = 1500;
/** The card slides this far down into place, in logical pixels. */
const CARD_SLIDE = 4;
/** Independent of the service: never hand an unbounded string to Clutter. */
const MAX_TITLE_CHARS = 256;

/**
 * Turn an untrusted signal value into one short paragraph. Iterating only as
 * far as the cap avoids copying a giant string before it can be rejected.
 */
function safeTitle(value) {
    if (typeof value !== 'string') return '';

    const chars = [];
    let inspected = 0;
    let pendingSpace = false;
    let truncated = false;
    for (const char of value) {
        // Count source characters too: a megabyte of whitespace must not make
        // the shell scan the whole value merely because it folds to one space.
        if (inspected++ >= MAX_TITLE_CHARS - 1) {
            truncated = true;
            break;
        }
        if (/\s/u.test(char)) {
            pendingSpace = chars.length > 0;
            continue;
        }
        if (pendingSpace) chars.push(' ');
        chars.push(char);
        pendingSpace = false;
    }
    return chars.join('') + (truncated ? '…' : '');
}

/**
 * St.BoxLayout traded `vertical` for `orientation` after GNOME 48, and this
 * extension supports 48 as well.
 */
function verticalBox(props) {
    const box = new St.BoxLayout(props);
    if ('orientation' in box) box.orientation = Clutter.Orientation.VERTICAL;
    else box.vertical = true;
    return box;
}

/**
 * Keep a fullscreen window redirected while the flash is up, or the compositor
 * hands the screen to that window and the flash is never drawn. Two spellings:
 * the older one went away after GNOME 48.
 */
function unredirect(allowed) {
    if (Meta.disable_unredirect_for_display !== undefined) {
        if (allowed) Meta.enable_unredirect_for_display(global.display);
        else Meta.disable_unredirect_for_display(global.display);
    } else if (global.compositor?.disable_unredirect !== undefined) {
        if (allowed) global.compositor.enable_unredirect();
        else global.compositor.disable_unredirect();
    }
}

/** The loudest an envelope ever gets, for the no-animation path. */
function peak(envelope) {
    return Math.max(envelope.from, ...envelope.steps.map(([, opacity]) => opacity));
}

export class FlashOverlay {
    constructor() {
        this._overlay = null;
        // Bumped for every flash, so a superseded animation stops where it is.
        this._generation = 0;
        // How many layers are still to play out before the overlay goes.
        this._running = 0;
        this._holdId = 0;
        this._unredirected = false;
    }

    /**
     * Draw one flash. `event` is the service's Flash payload:
     * `{style, intensity, palette, title, timer}`.
     */
    show(event) {
        const monitor = Main.layoutManager.primaryMonitor;
        if (!monitor) return;

        // A flash arriving on top of one still running replaces it outright.
        this.clear();
        const generation = this._generation;
        const look = INTENSITY[event.intensity] ?? INTENSITY.normal;
        const palette = PALETTE[event.palette] ?? PALETTE.blue;
        const panel = Math.max(0, Main.layoutManager.panelBox.height);

        const overlay = new St.Widget({
            reactive: false,
            can_focus: false,
            x: monitor.x,
            y: monitor.y,
            width: monitor.width,
            height: monitor.height,
            layout_manager: new Clutter.FixedLayout(),
        });
        // Belt and braces with `reactive: false`: never answer a pick either.
        Shell.util_set_hidden_from_pick(overlay, true);
        this._overlay = overlay;

        const layers = this._build(event, overlay, {look, palette, monitor, panel});

        Main.uiGroup.add_child(overlay);
        Main.uiGroup.set_child_above_sibling(overlay, null);
        if (!this._unredirected) {
            unredirect(false);
            this._unredirected = true;
        }

        // With animations off every ease finishes the instant it starts, which
        // would collapse the whole envelope into nothing on screen. Hold the
        // flash at its peak on a timer instead — the shell's own OSD does the
        // same — so the reminder still arrives.
        if (!St.Settings.get().enable_animations) {
            for (const {actor, envelope, motion} of layers) {
                actor.opacity = Math.round(peak(envelope) * 255);
                if (motion) actor[motion.property] = motion.to;
            }
            // `timeout_add`, not `timeout_add_once`: the latter is a gjs
            // override that only arrived in gjs 1.87, and this extension
            // supports GNOME 48, which ships 1.84.
            this._holdId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, STILL_MS, () => {
                this._holdId = 0;
                this.clear();
                return GLib.SOURCE_REMOVE;
            });
            return;
        }

        this._running = layers.length;
        for (const layer of layers) {
            // A step that finishes early enough to tear the overlay down would
            // leave the rest of this loop animating destroyed actors.
            if (generation !== this._generation) return;
            this._play(layer, generation);
        }
    }

    /** Take down whatever is on screen. Safe to call at any time, twice over. */
    clear() {
        // Bump first: a step that is already queued must find itself out of
        // date rather than tear anything down a second time.
        this._generation++;
        this._running = 0;
        if (this._holdId) {
            GLib.source_remove(this._holdId);
            this._holdId = 0;
        }
        if (this._overlay) {
            const overlay = this._overlay;
            this._overlay = null;
            overlay.remove_all_transitions();
            overlay.destroy();
        }
        if (this._unredirected) {
            unredirect(true);
            this._unredirected = false;
        }
    }

    destroy() {
        this.clear();
    }

    // ---- the layers -------------------------------------------------------

    /**
     * Put the style's layers into `overlay` and return what has to be played.
     * The card goes in last, so it sits over everything else.
     */
    _build(event, overlay, {look, palette, monitor, panel}) {
        const layers = [];
        const accent = alpha => `rgba(${palette.accent}, ${alpha})`;
        const full = (style, envelope) => {
            const actor = new St.Widget({
                reactive: false,
                x: 0,
                y: 0,
                width: monitor.width,
                height: monitor.height,
                style,
                layout_manager: new Clutter.FixedLayout(),
            });
            overlay.add_child(actor);
            layers.push({actor, envelope: ENVELOPE[envelope]});
            return actor;
        };

        switch (event.style) {
        case 'wash':
            full(`background-color: ${accent(look.wash)};`, 'wash');
            break;
        case 'wash2':
            full(`background-color: ${accent(look.wash)};`, 'wash2');
            break;
        case 'edges': {
            // The design draws this as one blurred inset shadow. St blurs
            // shadows on the CPU, in the paint path, with a kernel that both
            // costs seconds at full-screen size and washes out to a flat fill
            // at these radii — so the ring is a border and the glow is four
            // gradient strips, which the compositor draws for nothing.
            const group = full('', 'edges');
            group.add_child(new St.Widget({
                reactive: false,
                x: 0,
                y: 0,
                width: monitor.width,
                height: monitor.height,
                style: `border: ${look.edge}px solid ${accent(look.edgeAlpha)};`,
            }));
            this._glow(group, monitor, look.glow, palette, 0.7 * look.edgeAlpha, look.edge);
            break;
        }
        case 'edgesSoft': {
            const group = full('', 'breath');
            this._glow(group, monitor, look.glow * 2, palette, 0.8 * look.edgeAlpha, 0);
            break;
        }
        case 'topbar':
        case 'topbarBeam': {
            full(`background-color: ${accent(look.bar)};`, 'bar').height = panel;
            if (event.style === 'topbarBeam') {
                const beam = this._beam(overlay, {look, accent, monitor, panel});
                if (beam) layers.push(beam);
            }
            break;
        }
        }

        layers.push(this._card(event, overlay, {look, palette, monitor}));
        return layers;
    }

    /**
     * An inward glow along the four edges, as one gradient strip per side.
     * `depth` is how far in it reaches and `inset` how far in it starts, so the
     * ring the `edges` style draws is not painted over.
     *
     * Each strip runs the full length of its side, so they overlap at the
     * corners. That is deliberate: butting them together instead would leave a
     * seam where one strip's clear end meets the next one's solid start, and
     * the doubled corner is what an inward glow looks like anyway.
     */
    _glow(parent, monitor, depth, palette, alpha, inset) {
        const {width, height} = monitor;
        const room = Math.floor(Math.min(width, height) / 2) - inset;
        const deep = Math.max(1, Math.min(depth, room));
        const from = `rgba(${palette.accent}, ${alpha})`;
        // Fading to a fully transparent accent rather than to `transparent`,
        // which is transparent *black* and would dirty the middle of the fade.
        const to = `rgba(${palette.accent}, 0)`;
        const strip = (x, y, w, h, direction, inward) => {
            if (w <= 0 || h <= 0) return;
            parent.add_child(new St.Widget({
                reactive: false,
                x,
                y,
                width: w,
                height: h,
                style: `background-gradient-direction: ${direction};` +
                       `background-gradient-start: ${inward ? from : to};` +
                       `background-gradient-end: ${inward ? to : from};`,
            }));
        };
        const across = width - 2 * inset;
        const down = height - 2 * inset;
        strip(inset, inset, across, deep, 'vertical', true);
        strip(inset, height - inset - deep, across, deep, 'vertical', false);
        strip(inset, inset, deep, down, 'horizontal', true);
        strip(width - inset - deep, inset, deep, down, 'horizontal', false);
    }

    /** A beam from the bottom of the panel down to the middle of the screen. */
    _beam(overlay, {look, accent, monitor, panel}) {
        const height = Math.round(monitor.height / 2) - panel;
        if (height <= 0) return null;
        const beam = new St.Widget({
            reactive: false,
            x: Math.round((monitor.width - look.beam) / 2),
            y: panel,
            width: look.beam,
            height,
            style: `background-color: ${accent(1)};`,
        });
        // Grown from its top edge, so it drops towards the card.
        beam.set_pivot_point(0.5, 0);
        overlay.add_child(beam);
        return {
            actor: beam,
            envelope: ENVELOPE.beam,
            motion: {property: 'scale_y', from: 0, to: 1, fraction: 0.3},
        };
    }

    /** The card every style shows: NOW, the task, and its clock. */
    _card(event, overlay, {look, palette, monitor}) {
        const layer = new St.Widget({
            reactive: false,
            x: 0,
            y: 0,
            width: monitor.width,
            height: monitor.height,
            layout_manager: new Clutter.BinLayout(),
        });
        const card = verticalBox({
            style_class: 'qf-flash-card',
            x_align: Clutter.ActorAlign.CENTER,
            y_align: Clutter.ActorAlign.CENTER,
        });
        card.set_pivot_point(0.5, 0.5);
        card.set_scale(look.scale, look.scale);

        card.add_child(new St.Label({
            text: 'NOW',
            style_class: 'qf-flash-now',
            style: `color: ${palette.text};`,
            x_align: Clutter.ActorAlign.CENTER,
        }));

        const title = new St.Label({
            text: safeTitle(event.title),
            style_class: 'qf-flash-title',
            x_align: Clutter.ActorAlign.CENTER,
        });
        // A long title wraps rather than widening the card past its maximum,
        // and a bare URL breaks inside the word rather than running off it.
        title.clutter_text.line_wrap = true;
        title.clutter_text.line_wrap_mode = Pango.WrapMode.WORD_CHAR;
        title.clutter_text.ellipsize = Pango.EllipsizeMode.END;
        title.clutter_text.set_max_length(MAX_TITLE_CHARS);
        title.clutter_text.line_alignment = Pango.Alignment.CENTER;
        card.add_child(title);

        if (event.timer) {
            card.add_child(new St.Label({
                text: event.timer,
                style_class: 'qf-flash-timer',
                style: `color: ${palette.text};`,
                x_align: Clutter.ActorAlign.CENTER,
            }));
        }

        layer.add_child(card);
        overlay.add_child(layer);
        return {
            actor: layer,
            envelope: ENVELOPE.card,
            motion: {property: 'translation_y', from: -CARD_SLIDE, to: 0, fraction: 0.2},
        };
    }

    // ---- running the envelope ---------------------------------------------

    /**
     * Walk one layer through its envelope, one eased step at a time. Chained
     * rather than started together, because `ease` cancels whatever transition
     * is already running on the property it is given.
     */
    _play({actor, envelope, motion}, generation) {
        const {duration, from, steps} = envelope;
        actor.opacity = Math.round(from * 255);

        // Movement that runs alongside the fade. A different property, so the
        // two do not cancel one another.
        if (motion) {
            actor[motion.property] = motion.from;
            actor.ease({
                [motion.property]: motion.to,
                duration: Math.max(1, Math.round(motion.fraction * duration)),
                mode: MODE,
            });
        }

        let at = 0;
        const step = i => {
            if (generation !== this._generation) return;
            if (i >= steps.length) {
                this._played(generation);
                return;
            }
            const [mark, opacity] = steps[i];
            const ms = Math.max(1, Math.round((mark - at) * duration));
            at = mark;
            actor.ease({
                opacity: Math.round(opacity * 255),
                duration: ms,
                mode: MODE,
                onComplete: () => step(i + 1),
            });
        };
        step(0);
    }

    /** The overlay goes once its last layer has played out. */
    _played(generation) {
        if (generation !== this._generation) return;
        this._running--;
        if (this._running <= 0) this.clear();
    }
}
