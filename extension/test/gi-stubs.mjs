// Just enough of GNOME Shell to run the flash overlay outside one.
//
// The overlay itself cannot be exercised without a running shell, but its
// arithmetic can: which layers a style draws, where the glow strips and the
// beam land, and what is left on screen when animations are switched off.
// These stubs record what the overlay builds instead of drawing it.

/** Every actor built since the last `reset`, in creation order. */
export const built = [];

/** Swappable so a test can pretend the user has animations off. */
export const settings = {enable_animations: true};

class Actor {
    constructor(props = {}) {
        Object.assign(this, {
            opacity: 255,
            scale_y: 1,
            translation_y: 0,
            reactive: true,
            children: [],
            eases: [],
        }, props);
        built.push(this);
    }

    add_child(child) {
        this.children.push(child);
        child.parent = this;
    }

    set_child_above_sibling() {}
    remove_all_transitions() {}

    destroy() {
        this.destroyed = true;
    }

    set_pivot_point(x, y) {
        this.pivot = [x, y];
    }

    set_scale(x, y) {
        this.scale = [x, y];
    }

    /**
     * Land on the target values at once and run `onComplete` on the microtask
     * queue, so a chain of steps still unwinds one at a time rather than
     * recursing inside the call that started it.
     */
    ease(props) {
        this.eases.push({...props});
        const {onComplete, duration, mode, ...values} = props;
        void duration;
        void mode;
        Object.assign(this, values);
        if (onComplete) queueMicrotask(onComplete);
    }
}

export const Clutter = {
    AnimationMode: {EASE_IN_OUT_QUAD: 'ease-in-out-quad'},
    Orientation: {VERTICAL: 'vertical'},
    ActorAlign: {CENTER: 'center'},
    FixedLayout: class FixedLayout {},
    BinLayout: class BinLayout {},
};

export const St = {
    Widget: Actor,
    BoxLayout: class BoxLayout extends Actor {},
    Label: class Label extends Actor {
        constructor(props) {
            super(props);
            this.clutter_text = {
                set_max_length: max => {
                    this.clutter_text.max_length = max;
                },
            };
        }
    },
    Settings: {get: () => settings},
};

export const GLib = {
    PRIORITY_DEFAULT: 0,
    SOURCE_REMOVE: false,
    SOURCE_CONTINUE: true,
    /** The pending hold, for a test to fire by hand. */
    pendingHold: null,
    timeout_add(_priority, _ms, fn) {
        GLib.pendingHold = fn;
        return 1;
    },
    source_remove() {
        GLib.pendingHold = null;
    },
};

export const Meta = {};
export const Pango = {
    WrapMode: {WORD_CHAR: 'word-char'},
    EllipsizeMode: {NONE: 'none', END: 'end'},
    Alignment: {CENTER: 'center'},
};
export const Shell = {util_set_hidden_from_pick: () => {}};

export const layoutManager = {
    primaryMonitor: {x: 0, y: 0, width: 1920, height: 1080},
    panelBox: {height: 32},
};
export const uiGroup = new Actor();

/** Forget everything the last test built. */
export function reset() {
    built.length = 0;
    uiGroup.children.length = 0;
    settings.enable_animations = true;
    GLib.pendingHold = null;
    layoutManager.primaryMonitor = {x: 0, y: 0, width: 1920, height: 1080};
    layoutManager.panelBox = {height: 32};
    // The overlay reaches for the compositor to hold unredirection.
    globalThis.global = {compositor: {}, display: {}};
}
