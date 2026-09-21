// How wide the top bar's title and the menu may grow. Both take the width
// their text asks for, so nothing is cut short or wrapped to suit a fixed
// size; these are the limits the screen itself sets, as plain arithmetic so
// they can be checked without a shell. Every width is in physical pixels
// except the results, which are the logical pixels St's CSS is written in.

// Below this the title is not worth showing, however crowded the panel is.
const MIN_TITLE = 160;

/**
 * The widest the title may be before the panel has to cut a side box short.
 *
 * The panel gives its centre box the width it asks for, centres it on the work
 * area, and leaves each side box with what is left on its side, however much
 * that box needs. So the centre box may only grow until the fuller side runs
 * out: `centerOffset` is how far the work area's centre sits from the panel's
 * (twice over, as the panel works it out), and `gap` keeps a little air
 * between the boxes. Whatever else is in the centre box comes off the top.
 *
 * `startWidth` and `endWidth` are what the side boxes ask for, in the order
 * they sit on screen from the left.
 */
export function panelTitleRoom({panelWidth, centerOffset, startWidth, endWidth, centerWidth, titleWidth, gap, scale}) {
    const beforeStart = panelWidth + centerOffset - 2 * (startWidth + gap);
    const beforeEnd = panelWidth - centerOffset - 2 * (endWidth + gap);
    const others = centerWidth - titleWidth;
    const room = Math.floor((Math.min(beforeStart, beforeEnd) - others) / scale);
    return Math.max(MIN_TITLE, room);
}

/**
 * The widest the menu may be and still sit inside the work area. The shell
 * keeps it `edge` away from either side, and `margins` is whatever the menu's
 * own left and right margins add up to.
 */
export function menuMaxWidth({workAreaWidth, edge, margins, scale}) {
    return Math.max(0, Math.floor((workAreaWidth - 2 * edge - margins) / scale));
}
