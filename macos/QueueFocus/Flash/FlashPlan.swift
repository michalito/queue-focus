import CoreGraphics

// The screen flash the reminder asks for, as values: what to draw, where, and
// how each layer fades. Ported from the GNOME extension's flash.js, so both
// platforms draw the same flash. Lengths are points, in GNOME's coordinates:
// from the top left of the screen, y down.

/// A colour as flash.js writes one: red, green and blue out of 255, and an
/// alpha out of 1.
struct RGBA: Equatable {
    var red: Double
    var green: Double
    var blue: Double
    var alpha: Double

    init(_ red: Double, _ green: Double, _ blue: Double, _ alpha: Double = 1) {
        self.red = red
        self.green = green
        self.blue = blue
        self.alpha = alpha
    }

    /// `#rrggbb`.
    init(hex: UInt32) {
        self.init(Double(hex >> 16 & 0xff), Double(hex >> 8 & 0xff), Double(hex & 0xff))
    }

    func alpha(_ alpha: Double) -> RGBA {
        RGBA(red, green, blue, alpha)
    }
}

/// The two colour ways, as the engine names them.
struct FlashPalette: Equatable {
    let accent: RGBA
    /// NOW and the clock on the card.
    let text: RGBA

    static func of(_ palette: Palette) -> FlashPalette {
        switch palette {
        case .blue: FlashPalette(accent: RGBA(53, 132, 228), text: RGBA(hex: 0x62a0ea))
        case .orange: FlashPalette(accent: RGBA(230, 97, 0), text: RGBA(hex: 0xffa348))
        }
    }
}

/// How loud each intensity is.
struct FlashLook: Equatable {
    /// The wash's alpha.
    let wash: Double
    /// The edges' ring, and how far the glow reaches in.
    let edge: Double
    let glow: Double
    let edgeAlpha: Double
    /// The top bar's alpha, and the beam's width.
    let bar: Double
    let beam: Double
    /// The card's size.
    let scale: Double

    static func of(_ intensity: Intensity) -> FlashLook {
        switch intensity {
        case .subtle: FlashLook(wash: 0.14, edge: 4, glow: 24, edgeAlpha: 0.6, bar: 0.45, beam: 2, scale: 0.92)
        case .normal: FlashLook(wash: 0.26, edge: 8, glow: 56, edgeAlpha: 0.85, bar: 0.7, beam: 3, scale: 1)
        case .strong: FlashLook(wash: 0.42, edge: 14, glow: 100, edgeAlpha: 1, bar: 0.9, beam: 4, scale: 1.08)
        }
    }
}

/// An opacity envelope: where the run starts, then the opacity at each
/// fraction of the run. Every step eases in and out. A layer's own colour
/// already carries the intensity's alpha, so these run the full 0…1.
struct Envelope: Equatable {
    struct Step: Equatable {
        let at: Double
        let opacity: Double
    }

    /// Seconds.
    let duration: Double
    let from: Double
    let steps: [Step]

    private init(_ milliseconds: Double, from: Double, _ steps: [(Double, Double)]) {
        duration = milliseconds / 1000
        self.from = from
        self.steps = steps.map { Step(at: $0.0, opacity: $0.1) }
    }

    static let wash = Envelope(1600, from: 0, [(0.25, 1), (0.70, 1), (1, 0)])
    static let wash2 = Envelope(1700, from: 0, [(0.20, 1), (0.40, 0.45), (0.60, 1), (1, 0)])
    static let edges = Envelope(1700, from: 0, [(0.20, 1), (0.40, 0.5), (0.60, 1), (1, 0)])
    static let breath = Envelope(1800, from: 0, [(0.40, 1), (1, 0)])
    static let bar = Envelope(1700, from: 0, [(0.15, 1), (0.80, 1), (1, 0)])
    static let beam = Envelope(1700, from: 1, [(0.80, 1), (1, 0)])
    static let card = Envelope(1700, from: 0, [(0.20, 1), (0.80, 1), (1, 0)])

    /// The opacities in order, from the start.
    var values: [Double] { [from] + steps.map(\.opacity) }
    /// When each is reached, as fractions of the run.
    var keyTimes: [Double] { [0] + steps.map(\.at) }
    /// The loudest it gets: where a flash drawn without animation holds.
    var peak: Double { values.max() ?? from }
    /// Where it ends, so nothing jumps when the run is over.
    var end: Double { values.last ?? from }
}

/// Movement that runs alongside a layer's fade.
struct Motion: Equatable {
    enum Property: Equatable {
        case scaleY
        case translationY
    }

    let property: Property
    let from: Double
    let to: Double
    /// Over this fraction of the layer's run, from its start.
    let fraction: Double
}

/// Something a layer draws, in a frame of its own.
struct FlashPart: Equatable {
    enum Shape: Equatable {
        case fill(RGBA)
        /// A ring just inside the frame.
        case border(width: Double, RGBA)
        /// From `start` at the top, or the left, to `end`.
        case gradient(start: RGBA, end: RGBA, vertical: Bool)
        /// The card, in the middle of the frame.
        case card
    }

    /// In the layer's coordinates.
    let frame: CGRect
    let shape: Shape
}

/// One layer of the flash: what it draws, and the one envelope that fades all
/// of it together.
struct FlashLayer: Equatable {
    /// On the screen.
    let frame: CGRect
    let parts: [FlashPart]
    let envelope: Envelope
    var motion: Motion?
    /// What a scale grows from, as a fraction of the frame from its top left.
    var pivot = CGPoint(x: 0.5, y: 0.5)
}

/// What the card says, and how.
struct FlashCard: Equatable {
    let title: String
    /// Nil when the task's clock has not started.
    let timer: String?
    /// NOW and the clock.
    let color: RGBA
    let scale: Double
}

/// One flash, ready to draw: its layers from the bottom up, the card last.
struct FlashPlan: Equatable {
    /// How far the card slides down into place.
    static let cardSlide: Double = 4
    /// How long a flash drawn without animation stays up, in seconds.
    static let stillDuration: Double = 1.5

    /// The screen's size.
    let screen: CGSize
    let layers: [FlashLayer]
    let card: FlashCard

    /// Until the last layer has played out.
    var duration: Double {
        layers.map(\.envelope.duration).max() ?? 0
    }

    /// The flash for `event` on a screen of `size`, under a menu bar `menuBar`
    /// points tall.
    static func make(_ event: FlashEvent, screen size: CGSize, menuBar: Double) -> FlashPlan {
        let look = FlashLook.of(event.intensity)
        let palette = FlashPalette.of(event.palette)
        let accent = palette.accent
        let whole = CGRect(origin: .zero, size: size)
        let bar = max(0, menuBar)
        var layers: [FlashLayer] = []

        switch event.style {
        case .wash, .wash2:
            layers.append(FlashLayer(frame: whole, parts: [FlashPart(frame: whole, shape: .fill(accent.alpha(look.wash)))],
                                     envelope: event.style == .wash ? .wash : .wash2))
        case .edges:
            let ring = FlashPart(frame: whole, shape: .border(width: look.edge, accent.alpha(look.edgeAlpha)))
            let glow = glow(size, depth: look.glow, accent: accent, alpha: 0.7 * look.edgeAlpha, inset: look.edge)
            layers.append(FlashLayer(frame: whole, parts: [ring] + glow, envelope: .edges))
        case .edgesSoft:
            let glow = glow(size, depth: look.glow * 2, accent: accent, alpha: 0.8 * look.edgeAlpha, inset: 0)
            layers.append(FlashLayer(frame: whole, parts: glow, envelope: .breath))
        case .topbar, .topbarBeam:
            let band = CGRect(x: 0, y: 0, width: size.width, height: bar)
            layers.append(FlashLayer(frame: band, parts: [FlashPart(frame: CGRect(origin: .zero, size: band.size), shape: .fill(accent.alpha(look.bar)))],
                                     envelope: .bar))
            if event.style == .topbarBeam, let beam = beam(size, width: look.beam, accent: accent, below: bar) {
                layers.append(beam)
            }
        }

        // The card every style shows, over everything else.
        layers.append(FlashLayer(frame: whole, parts: [FlashPart(frame: whole, shape: .card)], envelope: .card,
                                 motion: Motion(property: .translationY, from: -cardSlide, to: 0, fraction: 0.2)))
        let card = FlashCard(title: safeTitle(event.title), timer: event.timer.isEmpty ? nil : event.timer,
                             color: palette.text, scale: look.scale)
        return FlashPlan(screen: size, layers: layers, card: card)
    }

    /// An inward glow along the four edges, one gradient strip per side.
    /// `depth` is how far in it reaches and `inset` how far in it starts, so
    /// the ring the `edges` style draws is not painted over.
    ///
    /// Each strip runs the whole of its side, so they overlap at the corners:
    /// butted together, one strip's clear end would meet the next one's solid
    /// start in a seam, and the doubled corner is what an inward glow looks
    /// like anyway.
    private static func glow(_ size: CGSize, depth: Double, accent: RGBA, alpha: Double, inset: Double) -> [FlashPart] {
        let width = Double(size.width)
        let height = Double(size.height)
        let room = (min(width, height) / 2).rounded(.down) - inset
        let deep = max(1, min(depth, room))
        // To a clear accent, not to clear black, which would dirty the fade.
        let solid = accent.alpha(alpha)
        let clear = accent.alpha(0)
        var strips: [FlashPart] = []
        func strip(_ x: Double, _ y: Double, _ w: Double, _ h: Double, vertical: Bool, inward: Bool) {
            guard w > 0, h > 0 else { return }
            strips.append(FlashPart(frame: CGRect(x: x, y: y, width: w, height: h),
                                    shape: .gradient(start: inward ? solid : clear, end: inward ? clear : solid, vertical: vertical)))
        }
        let across = width - 2 * inset
        let down = height - 2 * inset
        strip(inset, inset, across, deep, vertical: true, inward: true)
        strip(inset, height - inset - deep, across, deep, vertical: true, inward: false)
        strip(inset, inset, deep, down, vertical: false, inward: true)
        strip(width - inset - deep, inset, deep, down, vertical: false, inward: false)
        return strips
    }

    /// A beam from the bottom of the menu bar down to the middle of the
    /// screen, grown from its top edge so it drops towards the card.
    private static func beam(_ size: CGSize, width: Double, accent: RGBA, below bar: Double) -> FlashLayer? {
        let height = (Double(size.height) / 2).rounded() - bar
        guard height > 0 else { return nil }
        let frame = CGRect(x: ((Double(size.width) - width) / 2).rounded(), y: bar, width: width, height: height)
        return FlashLayer(frame: frame, parts: [FlashPart(frame: CGRect(origin: .zero, size: frame.size), shape: .fill(accent))],
                          envelope: .beam, motion: Motion(property: .scaleY, from: 0, to: 1, fraction: 0.3),
                          pivot: CGPoint(x: 0.5, y: 0))
    }
}

/// The most of a title the card shows, in characters, as the core caps one.
private let maxCardTitle = 256

/// An untrusted title as one short paragraph: runs of whitespace fold to one
/// space, none leads or trails, and a title cut short ends in `…`. As
/// flash.js's `safeTitle`, it looks at no more of the title than it can show,
/// so a megabyte of whitespace costs no more than a short title.
func safeTitle(_ value: String) -> String {
    var title = String.UnicodeScalarView()
    var inspected = 0
    var pendingSpace = false
    var truncated = false
    for scalar in value.unicodeScalars {
        if inspected >= maxCardTitle - 1 {
            truncated = true
            break
        }
        inspected += 1
        // JavaScript's `\s`: Unicode's white space but the next-line
        // control, and the byte order mark.
        if scalar.properties.isWhitespace && scalar != "\u{0085}" || scalar == "\u{FEFF}" {
            pendingSpace = !title.isEmpty
            continue
        }
        if pendingSpace {
            title.append(" ")
        }
        title.append(scalar)
        pendingSpace = false
    }
    return String(title) + (truncated ? "…" : "")
}
