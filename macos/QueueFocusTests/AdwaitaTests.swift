import AppKit
import Testing
@testable import QueueFocus

/// The GNOME look's colour arithmetic: GTK's mix, and text on the accent's
/// panel that still reads whatever accent the system has, at rest and under
/// the pointer.
@Suite struct AdwaitaTests {
    private static let light = (fg: NSColor(srgbRed: 0, green: 0, blue: 0, alpha: 0.8), card: NSColor.white,
                                grey: NSColor(srgbRed: 0.38, green: 0.38, blue: 0.38, alpha: 1))
    private static let dark = (fg: NSColor.white, card: Adw.rgb(0x353535),
                               grey: NSColor(srgbRed: 0.68, green: 0.68, blue: 0.68, alpha: 1))

    /// The panel at rest and under a flat button's hover wash.
    private func states(_ accent: NSColor, dark: Bool) -> [NSColor] {
        let scheme = dark ? Self.dark : Self.light
        let panel = scheme.card.mixed(with: accent, 0.12)
        let wash = scheme.fg.withAlphaComponent(scheme.fg.alphaComponent * 0.07)
        return [panel, wash.composited(over: panel)]
    }

    @Test func contrastIsWCAGs() {
        #expect(abs(NSColor.black.contrast(on: .white) - 21) < 0.01)
        #expect(abs(NSColor.white.contrast(on: .white) - 1) < 0.01)
        // Half-transparent black over white is mid grey.
        #expect(abs(NSColor(srgbRed: 0, green: 0, blue: 0, alpha: 0.5).contrast(on: .white) - 3.95) < 0.1)
    }

    /// GTK 4.14's mix is premultiplied: the translucent text colour
    /// weighs by its alpha.
    @Test func mixingIsPremultipliedAsGTKs() {
        let mixed = Adw.rgb(0x3584E4).mixed(with: Self.light.fg, 0.34)
        let alpha = 0.66 + 0.34 * 0.8
        #expect(abs(mixed.alphaComponent - alpha) < 0.001)
        #expect(abs(mixed.redComponent - 0.66 * 0x35 / 255 / alpha) < 0.001)
        // Between two opaque colours it is the plain mix.
        let opaque = NSColor.white.mixed(with: Adw.rgb(0x3584E4), 0.22)
        #expect(abs(opaque.greenComponent - (0.78 + 0.22 * 0x84 / 255)) < 0.001)
    }

    /// In the light GNOME's ink for the tags reads on the panel at rest;
    /// ours is that, or at most two steps nearer the text colour where the
    /// timer's hover wash needs it.
    @Test(arguments: [0x3584E4, 0xE66100])
    func theTagsKeepCloseToGNOMEsInkInTheLight(hex: Int) {
        let accent = Adw.rgb(hex)
        let backgrounds = states(accent, dark: false)
        #expect(accent.mixed(with: Self.light.fg, 0.34).contrast(on: backgrounds[0]) >= 4.5)
        let ink = AdwAccent.ink(from: accent, foreground: Self.light.fg, on: backgrounds)
        #expect([0.34, 0.40, 0.46].contains { ink == accent.mixed(with: Self.light.fg, $0) })
    }

    /// The tags' and the system's accents, light and dark: the clock and
    /// NOW read on the panel, and so does the panel's grey (a paused clock,
    /// the untagged chip, the empty panel), with the pointer on them or not.
    @Test(arguments: [0x3584E4, 0xE66100, 0xFFCC00, 0x28CD41, 0xFF9500, 0xA550A7, 0x8C8C8C])
    func textOnThePanelReads(hex: Int) {
        let accent = Adw.rgb(hex)
        for dark in [false, true] {
            let scheme = dark ? Self.dark : Self.light
            let backgrounds = states(accent, dark: dark)
            let ink = AdwAccent.ink(from: accent, foreground: scheme.fg, on: backgrounds)
            let grey = AdwAccent.readable(scheme.grey, toward: scheme.fg, from: 0, on: backgrounds)
            for background in backgrounds {
                #expect(ink.contrast(on: background) >= 4.5, "ink of \(String(hex, radix: 16)), dark: \(dark)")
                #expect(grey.contrast(on: background) >= 4.5, "grey on \(String(hex, radix: 16)), dark: \(dark)")
            }
        }
    }

    /// What the stepping is for: a bright accent at GNOME's mix does not
    /// read.
    @Test func aBrightAccentNeedsMoreThanGNOMEsMix() {
        let yellow = Adw.rgb(0xFFCC00)
        let rest = states(yellow, dark: false)[0]
        #expect(yellow.mixed(with: Self.light.fg, 0.34).contrast(on: rest) < 4.5)
        // And the dark work ink under the pointer.
        let blue = Adw.rgb(0x3584E4)
        let hover = states(blue, dark: true)[1]
        #expect(blue.mixed(with: Self.dark.fg, 0.34).contrast(on: hover) < 4.5)
    }
}

/// The chosen segment's white words on its lift, wherever the control sits.
@MainActor
@Suite struct SegmentedLiftTests {
    @Test(arguments: [0x3584E4, 0xE66100, 0xFFCC00, 0x28CD41, 0xFF9500, 0xA550A7, 0x8C8C8C])
    func theChosenSegmentReadsOnADarkHeaderBar(hex: Int) {
        let header = Adw.rgb(0x303030).mixed(with: Adw.rgb(hex), 0.22)
        let alpha = Segmented<Int>.liftAlpha(over: header)
        let tray = NSColor.white.withAlphaComponent(0.08).composited(over: header)
        let lift = NSColor.white.withAlphaComponent(alpha).composited(over: tray)
        #expect(NSColor.white.contrast(on: lift) >= 4.5)
        #expect(alpha > 0, "still lifted out of the tray")
    }

    /// On a card, and on the blue header, GNOME's 20% stays.
    @Test func gnomesLiftStaysWhereItReads() {
        #expect(Segmented<Int>.liftAlpha(over: Adw.rgb(0x353535)) == 0.2)
        #expect(Segmented<Int>.liftAlpha(over: Adw.rgb(0x303030).mixed(with: Adw.rgb(0x3584E4), 0.22)) == 0.2)
    }
}
