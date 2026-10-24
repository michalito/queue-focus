import AppKit
import SwiftUI

/// The GNOME app's look, as libadwaita 1.5 draws it on Ubuntu 24.04: its
/// colours, the sizes in the app's stylesheet (`style.css`), and the few
/// controls drawn its way rather than the Mac's. Colour is kept for the
/// task's tag: it tints the header bar, the add field's band, the Now panel
/// and the Side cards.
enum Adw {
    /// 1em. GNOME's 11 pt interface font is 14.67 px; SF at 14 pt sets a line
    /// about as long as Ubuntu Sans does there.
    static let em: CGFloat = 14
    /// The header bar's height: GNOME's is 47 px; 44 centres its controls
    /// within 2 pt of the window's own buttons.
    static let headerHeight: CGFloat = 44
    /// Controls, and the corners of anything smaller than a card.
    static let radius: CGFloat = 6
    static let cardRadius: CGFloat = 12

    static func font(_ size: CGFloat = 1, _ weight: Font.Weight = .regular, design: Font.Design = .default) -> Font {
        .system(size: em * size, weight: weight, design: design)
    }

    // MARK: libadwaita 1.5's named colours

    static let windowNS = dynamic { $0 ? rgb(0x242424) : rgb(0xFAFAFA) }
    static let headerBarNS = dynamic { $0 ? rgb(0x303030) : .white }
    /// Cards: white, and white at 8% over the dark window.
    static let cardNS = dynamic { $0 ? rgb(0x353535) : .white }
    /// The hairline round a card and between the rows in it.
    static let cardShadeNS = dynamic { NSColor(srgbRed: 0, green: 0, blue: 6 / 255, alpha: $0 ? 0.36 : 0.07) }
    static let foregroundNS = dynamic { $0 ? .white : NSColor(srgbRed: 0, green: 0, blue: 0, alpha: 0.8) }
    static let popoverNS = dynamic { $0 ? rgb(0x383838) : .white }

    static let window = Color(nsColor: windowNS)
    static let card = Color(nsColor: cardNS)
    static let cardShade = Color(nsColor: cardShadeNS)
    static let foreground = Color(nsColor: foregroundNS)
    static let popover = Color(nsColor: popoverNS)
    static let destructive = Color(nsColor: dynamic { $0 ? rgb(0xFF7B63) : rgb(0xC01C28) })
    /// A card under the pointer: the card a little towards the text colour.
    static let cardHover = Color(nsColor: dynamic { _ in cardNS.mixed(with: foregroundNS, 0.12) })
    /// GNOME's text at 55%, in a grey that still reads at 4.5 to 1.
    static let dim = Color.readableGrey

    /// A wash of the text colour, as Adwaita's buttons and entries are
    /// filled: `alpha(currentColor, amount)`.
    static func wash(_ amount: Double) -> Color {
        foreground.opacity(amount)
    }

    /// A colour for light and one for dark, worked out for whatever
    /// appearance draws it.
    static func dynamic(_ make: @escaping (_ dark: Bool) -> NSColor) -> NSColor {
        NSColor(name: nil) { appearance in
            var color = NSColor.clear
            appearance.performAsCurrentDrawingAppearance {
                color = make(appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua)
            }
            return color
        }
    }

    static func rgb(_ hex: Int) -> NSColor {
        NSColor(srgbRed: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
                blue: CGFloat(hex & 0xFF) / 255, alpha: 1)
    }
}

extension NSColor {
    /// The contrast ratio of this colour, laid over `background`, against
    /// it (WCAG's relative luminance).
    func contrast(on background: NSColor) -> CGFloat {
        guard let a = usingColorSpace(.sRGB), let b = background.usingColorSpace(.sRGB) else { return 1 }
        func over(_ x: CGFloat, _ y: CGFloat) -> CGFloat { x * a.alphaComponent + y * (1 - a.alphaComponent) }
        func linear(_ c: CGFloat) -> CGFloat { c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4) }
        func luminance(_ r: CGFloat, _ g: CGFloat, _ b: CGFloat) -> CGFloat {
            0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
        }
        let front = luminance(over(a.redComponent, b.redComponent), over(a.greenComponent, b.greenComponent),
                              over(a.blueComponent, b.blueComponent))
        let back = luminance(b.redComponent, b.greenComponent, b.blueComponent)
        return (max(front, back) + 0.05) / (min(front, back) + 0.05)
    }

    /// This colour laid over an opaque `background`.
    func composited(over background: NSColor) -> NSColor {
        guard let a = usingColorSpace(.sRGB), let b = background.usingColorSpace(.sRGB) else { return self }
        func over(_ x: CGFloat, _ y: CGFloat) -> CGFloat { x * a.alphaComponent + y * (1 - a.alphaComponent) }
        return NSColor(srgbRed: over(a.redComponent, b.redComponent), green: over(a.greenComponent, b.greenComponent),
                       blue: over(a.blueComponent, b.blueComponent), alpha: 1)
    }

    /// GTK's `mix(self, other, amount)`: `amount` of the way to `other`,
    /// in premultiplied sRGB as GTK 4.14 mixes, so a translucent colour
    /// (the light theme's text) weighs by its alpha.
    func mixed(with other: NSColor, _ amount: CGFloat) -> NSColor {
        guard let a = usingColorSpace(.sRGB), let b = other.usingColorSpace(.sRGB) else { return self }
        let alpha = a.alphaComponent + (b.alphaComponent - a.alphaComponent) * amount
        guard alpha > 0 else { return .clear }
        func mix(_ x: CGFloat, _ y: CGFloat) -> CGFloat {
            (x * a.alphaComponent * (1 - amount) + y * b.alphaComponent * amount) / alpha
        }
        return NSColor(srgbRed: mix(a.redComponent, b.redComponent), green: mix(a.greenComponent, b.greenComponent),
                       blue: mix(a.blueComponent, b.blueComponent), alpha: alpha)
    }
}

/// The accent the window takes from the current task's tag, as the GNOME
/// app sets `--accent-bg-color` per window: blue for work, orange for
/// personal, and the system's accent while the task has no tag or there is
/// no current task.
struct AdwAccent {
    /// `accent_bg_color`.
    let background: Color
    /// `accent_color`, for text: the accent a third of the way to the text
    /// colour.
    let ink: Color
    /// The header bar and the add field's band.
    let headerBar: Color
    let headerBarNS: NSColor
    /// The Now banner and panel.
    let panel: Color
    /// The Board's Side cards.
    let sideCard: Color
    /// Grey text on the panel: the readable grey, nearer the text colour
    /// where a bright accent's tint needs it.
    let dim: Color

    private init(_ tag: TaskTag?) {
        let background = Adw.dynamic { _ in
            switch tag {
            case .work: Adw.rgb(0x3584E4)
            case .personal: Adw.rgb(0xE66100)
            case nil: NSColor.controlAccentColor.usingColorSpace(.sRGB) ?? Adw.rgb(0x3584E4)
            }
        }
        self.background = Color(nsColor: background)
        let panel = Adw.dynamic { _ in Adw.cardNS.mixed(with: background, 0.12) }
        ink = Color(nsColor: Adw.dynamic { _ in
            Self.ink(from: background, foreground: Adw.foregroundNS, on: Self.panelStates(panel))
        })
        dim = Color(nsColor: Adw.dynamic { _ in
            Self.readable(.readableGrey, toward: Adw.foregroundNS, from: 0, on: Self.panelStates(panel))
        })
        headerBarNS = Adw.dynamic { _ in Adw.headerBarNS.mixed(with: background, 0.22) }
        headerBar = Color(nsColor: headerBarNS)
        self.panel = Color(nsColor: panel)
        sideCard = Color(nsColor: Adw.dynamic { _ in Adw.cardNS.mixed(with: background, 0.05) })
    }

    /// The panel as text on it is seen: at rest, and under a flat button's
    /// hover wash (the timer is one).
    static func panelStates(_ panel: NSColor) -> [NSColor] {
        let panel = panel.usingColorSpace(.sRGB) ?? .white
        let foreground = Adw.foregroundNS.usingColorSpace(.sRGB) ?? .black
        return [panel, foreground.withAlphaComponent(foreground.alphaComponent * 0.07).composited(over: panel)]
    }

    /// Text in the accent: GNOME's third of the way to the text colour, and
    /// further while that does not read at 4.5 to 1 on every one of
    /// `backgrounds`, as a bright system accent (yellow, say) needs.
    static func ink(from background: NSColor, foreground: NSColor, on backgrounds: [NSColor]) -> NSColor {
        readable(background, toward: foreground, from: 0.34, on: backgrounds)
    }

    /// `color` moved towards `foreground`, from `amount` on, until it reads
    /// at 4.5 to 1 on every one of `backgrounds`.
    static func readable(_ color: NSColor, toward foreground: NSColor, from amount: CGFloat,
                         on backgrounds: [NSColor]) -> NSColor {
        var amount = amount
        var mixed = color.mixed(with: foreground, amount)
        while backgrounds.contains(where: { mixed.contrast(on: $0) < 4.5 }), amount < 1 {
            amount = min(1, amount + 0.06)
            mixed = color.mixed(with: foreground, amount)
        }
        return mixed
    }

    private static let work = AdwAccent(.work)
    private static let personal = AdwAccent(.personal)
    private static let system = AdwAccent(nil)

    static func of(_ tag: TaskTag?) -> AdwAccent {
        switch tag {
        case .work: work
        case .personal: personal
        case nil: system
        }
    }
}

extension EnvironmentValues {
    /// The window's accent, from its current task's tag.
    @Entry var adwAccent = AdwAccent.of(nil)
}

// MARK: Text

extension View {
    /// A bucket's name over its tasks: small capitals, spaced, a little faded.
    func bucketHeaderStyle() -> some View {
        font(Adw.font(0.8, .bold)).tracking(0.06 * 0.8 * Adw.em).textCase(.uppercase)
    }
}

/// The count beside a bucket's name.
struct SectionCount: View {
    let count: Int

    var body: some View {
        Text("\(count)")
            .font(Adw.font(0.75))
            .monospacedDigit()
            .foregroundStyle(Adw.dim)
    }
}

/// `empty`, in a bucket with nothing in it.
struct EmptyLabel: View {
    var text = "empty"
    var color = Adw.dim

    var body: some View {
        Text(text)
            .font(Adw.font().italic())
            .foregroundStyle(color)
    }
}

// MARK: Buttons

/// Adwaita's buttons: `.flat` shows only under the pointer, `.filled`
/// always has its wash. Both are 6 pt rounded rectangles.
struct AdwButtonStyle: ButtonStyle {
    enum Kind { case flat, filled }
    var kind: Kind = .flat
    var padding = EdgeInsets(top: 4, leading: 10, bottom: 4, trailing: 10)
    var radius = Adw.radius
    /// Faded until the pointer or the keyboard is on the row it is in, as
    /// the row buttons are; 1 for none.
    var restingOpacity: Double = 1
    /// A flat button's wash while pressed. The timer keeps its hover wash,
    /// so the accent's words on it read pressed too.
    var pressedWash = 0.16

    func makeBody(configuration: Configuration) -> some View {
        StyledButton(configuration: configuration, style: self)
    }

    private struct StyledButton: View {
        let configuration: Configuration
        let style: AdwButtonStyle
        @Environment(\.isEnabled) private var isEnabled
        @State private var hovering = false

        var body: some View {
            configuration.label
                .padding(style.padding)
                .contentShape(RoundedRectangle(cornerRadius: style.radius))
                .background(Adw.wash(wash), in: RoundedRectangle(cornerRadius: style.radius))
                .opacity(isEnabled ? (hovering || configuration.isPressed ? 1 : style.restingOpacity) : 0.5)
                .onHover { hovering = $0 }
        }

        private var wash: Double {
            switch (style.kind, configuration.isPressed, hovering && isEnabled) {
            case (.flat, true, _): style.pressedWash
            case (.flat, false, true): 0.07
            case (.flat, false, false): 0
            case (.filled, true, _): 0.24
            case (.filled, false, true): 0.15
            case (.filled, false, false): 0.10
            }
        }
    }
}

extension ButtonStyle where Self == AdwButtonStyle {
    static var adwFlat: AdwButtonStyle { AdwButtonStyle() }
    static var adwFilled: AdwButtonStyle { AdwButtonStyle(kind: .filled) }

    /// A flat button for one glyph, which sets its own size.
    static func adwIcon(resting: Double = 1) -> AdwButtonStyle {
        AdwButtonStyle(padding: EdgeInsets(), restingOpacity: resting)
    }
}

/// An icon button's glyph, centred in a square of `size`.
struct IconGlyph: View {
    let systemName: String
    var size: CGFloat = 24
    var weight: Font.Weight = .semibold

    var body: some View {
        Image(systemName: systemName)
            .font(.system(size: Adw.em * 0.95, weight: weight))
            .frame(width: size, height: size)
    }
}

/// GNOME's `view-more-symbolic`: three dots stacked.
struct MoreGlyph: View {
    var size: CGFloat = 24

    var body: some View {
        VStack(spacing: 2.5) {
            ForEach(0..<3, id: \.self) { _ in
                Circle().frame(width: 3.5, height: 3.5)
            }
        }
        .frame(width: size, height: size)
    }
}

// MARK: The view switcher and segmented choices

/// Adwaita's linked toggle buttons, as the app's `.view-switch` dresses
/// them: a faint tray, and the chosen one lifted out of it in bold.
struct Segmented<Value: Hashable>: View {
    let options: [(value: Value, label: String)]
    let selection: Value
    var horizontalPadding: CGFloat = 14
    var help: (Value) -> String? = { _ in nil }
    var identifier: (Value) -> String? = { _ in nil }
    /// What the control sits on: a card, or the header bar.
    var surface: NSColor = Adw.cardNS
    let choose: (Value) -> Void

    var body: some View {
        HStack(spacing: 0) {
            ForEach(options, id: \.value) { option in
                let chosen = option.value == selection
                Button {
                    choose(option.value)
                } label: {
                    Text(option.label)
                        .font(Adw.font(0.85, chosen ? .bold : .regular))
                        .foregroundStyle(Adw.foreground)
                        // GNOME's 0.7 reads under 4.5 to 1 on a header bar
                        // tinted by a bright accent; 0.85 reads.
                        .opacity(chosen ? 1 : 0.85)
                        .padding(.horizontal, horizontalPadding)
                        .padding(.vertical, 3)
                        .frame(minHeight: 22)
                        .background(chosen ? lifted : .clear, in: RoundedRectangle(cornerRadius: 5))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help(help(option.value) ?? "")
                .accessibilityAddTraits(chosen ? [.isSelected] : [])
                .accessibilityIdentifier(identifier(option.value) ?? "")
            }
        }
        .padding(2)
        .background(Adw.wash(0.08), in: RoundedRectangle(cornerRadius: Adw.radius))
        .fixedSize()
        .accessibilityElement(children: .contain)
    }

    /// The chosen one's lift: white in the light; in the dark, white at
    /// GNOME's 20%, less where the white words on it would not read, as on
    /// a header bar tinted by a bright accent.
    private var lifted: Color {
        let surface = surface
        return Color(nsColor: Adw.dynamic { dark in
            dark ? NSColor.white.withAlphaComponent(Self.liftAlpha(over: surface)) : .white
        })
    }

    /// The most of GNOME's 20% white lift, over the tray's 8% wash on
    /// `surface`, under which white text still reads at 4.5 to 1.
    static func liftAlpha(over surface: NSColor) -> CGFloat {
        let tray = NSColor.white.withAlphaComponent(0.08).composited(over: surface.usingColorSpace(.sRGB) ?? .black)
        var alpha: CGFloat = 0.2
        while alpha > 0, NSColor.white.contrast(on: NSColor.white.withAlphaComponent(alpha).composited(over: tray)) < 4.5 {
            alpha = max(0, alpha - 0.02)
        }
        return alpha
    }
}

// MARK: Sliders

/// GNOME's scale: a thin trough filled with the accent up to a round white
/// knob. It moves in whole steps, by the pointer, the arrow keys and
/// VoiceOver alike; AppKit's slider would draw a tick for every step, and
/// GNOME's has a few labelled marks, which the row draws itself.
struct AdwSlider: View {
    /// The knob's width: the marks under the slider are inset by half.
    static let knob: CGFloat = 20

    @Binding var value: Double
    let range: ClosedRange<Double>
    let step: Double
    let label: String
    let valueDescription: String
    @Environment(\.adwAccent) private var accent
    @Environment(\.isEnabled) private var isEnabled
    @FocusState private var focused: Bool

    var body: some View {
        GeometryReader { geometry in
            let span = max(1, geometry.size.width - Self.knob)
            let fraction = (value - range.lowerBound) / (range.upperBound - range.lowerBound)
            ZStack(alignment: .leading) {
                Capsule().fill(Adw.wash(0.15))
                    .frame(width: span, height: 4)
                    .offset(x: Self.knob / 2)
                Capsule().fill(accent.background)
                    .frame(width: span * fraction, height: 4)
                    .offset(x: Self.knob / 2)
                Circle()
                    .fill(.white)
                    .overlay(Circle().strokeBorder(Color.black.opacity(0.12)))
                    .shadow(color: .black.opacity(0.2), radius: 1, y: 1)
                    .overlay {
                        if focused {
                            Circle().strokeBorder(accent.background.opacity(0.5), lineWidth: 2).padding(-3)
                        }
                    }
                    .frame(width: Self.knob, height: Self.knob)
                    .offset(x: span * fraction)
            }
            .frame(width: geometry.size.width, height: Self.knob)
            .contentShape(Rectangle())
            .gesture(DragGesture(minimumDistance: 0).onChanged { drag in
                set(range.lowerBound + (drag.location.x - Self.knob / 2) / span * (range.upperBound - range.lowerBound))
            })
        }
        .frame(height: Self.knob)
        .opacity(isEnabled ? 1 : 0.5)
        .focusable()
        .focused($focused)
        .focusEffectDisabled()
        .onKeyPress(keys: [.leftArrow, .downArrow, .rightArrow, .upArrow]) { press in
            nudge(press.key == .leftArrow || press.key == .downArrow ? -1 : 1)
            return .handled
        }
        // To VoiceOver, AppKit's own slider, in the same steps.
        .accessibilityRepresentation {
            Slider(value: Binding(get: { value }, set: { set($0) }), in: range, step: step) {
                Text(label)
            }
            .accessibilityValue(valueDescription)
        }
    }

    private func nudge(_ steps: Double) {
        set(value + steps * step)
    }

    /// To the nearest step inside the range.
    private func set(_ raw: Double) {
        let stepped = range.lowerBound + ((raw - range.lowerBound) / step).rounded() * step
        let clamped = min(max(stepped, range.lowerBound), range.upperBound)
        if clamped != value { value = clamped }
    }
}

// MARK: Switches

/// Adwaita's switch, with the app's 6 pt corners: a rounded rectangle
/// in the accent when on.
struct AdwSwitchStyle: ToggleStyle {
    func makeBody(configuration: Configuration) -> some View {
        SwitchRow(configuration: configuration)
    }

    private struct SwitchRow: View {
        let configuration: Configuration
        @Environment(\.adwAccent) private var accent
        @Environment(\.isEnabled) private var isEnabled
        @Environment(\.colorScheme) private var colorScheme

        var body: some View {
            // The whole row is the switch, as in an Adwaita switch row.
            Button {
                withAnimation(.easeOut(duration: 0.15)) { configuration.isOn.toggle() }
            } label: {
                HStack(spacing: 12) {
                    configuration.label
                        .frame(maxWidth: .infinity, alignment: .leading)
                    ZStack(alignment: configuration.isOn ? .trailing : .leading) {
                        RoundedRectangle(cornerRadius: Adw.radius)
                            .fill(configuration.isOn ? accent.background : Adw.wash(colorScheme == .dark ? 0.2 : 0.15))
                        RoundedRectangle(cornerRadius: 4)
                            .fill(colorScheme == .dark && !configuration.isOn ? Color(white: 0.82) : .white)
                            .shadow(color: .black.opacity(0.2), radius: 1, y: 1)
                            .frame(width: 20, height: 20)
                            .padding(3)
                    }
                    .frame(width: 46, height: 26)
                    .opacity(isEnabled ? 1 : 0.5)
                    .accessibilityHidden(true)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityAddTraits(.isToggle)
            .accessibilityValue(configuration.isOn ? "on" : "off")
        }
    }
}

// MARK: Entries

/// Adwaita's entry: a filled, borderless field that rings in the accent
/// while it has the keyboard.
struct AdwEntry: ViewModifier {
    var font = Adw.font(0.85)
    var padding = EdgeInsets(top: 7, leading: 9, bottom: 7, trailing: 9)
    var focused: Bool
    @Environment(\.adwAccent) private var accent

    func body(content: Content) -> some View {
        content
            .textFieldStyle(.plain)
            .font(font)
            .padding(padding)
            .background(Adw.wash(0.1), in: RoundedRectangle(cornerRadius: Adw.radius))
            .overlay {
                if focused {
                    RoundedRectangle(cornerRadius: Adw.radius)
                        .strokeBorder(accent.background.opacity(0.5), lineWidth: 2)
                }
            }
    }
}

// MARK: Cards

extension View {
    /// A libadwaita card: the card colour, a hairline round it, 12 pt
    /// corners.
    func adwCard(_ fill: Color = Adw.card, radius: CGFloat = Adw.cardRadius) -> some View {
        background(fill, in: RoundedRectangle(cornerRadius: radius))
            .overlay(RoundedRectangle(cornerRadius: radius).strokeBorder(Adw.cardShade, lineWidth: 1))
            .clipShape(RoundedRectangle(cornerRadius: radius))
    }
}

/// A hairline across a card, between its sections or rows.
struct Hairline: View {
    var color = Adw.cardShade

    var body: some View {
        Rectangle().fill(color).frame(height: 1).accessibilityHidden(true)
    }
}

// MARK: The window's chrome

/// The window's title bar given over to the content, as GNOME's header bar
/// is drawn by the app: the title is hidden (it still names the window to
/// VoiceOver and the Window menu), and the Mac's own buttons stay at the
/// left. An empty compact toolbar makes the title bar tall enough to centre
/// them in the header bar; it has no items, so nothing rebuilds it as the
/// clock ticks. (A full-height one would take the clicks meant for the
/// header bar's controls.) The scene must also ask for
/// `.windowStyle(.hiddenTitleBar)`: SwiftUI puts its own title bar back
/// over the header bar whenever the window updates.
struct HeaderBarWindow: ViewModifier {
    func body(content: Content) -> some View {
        content
            .ignoresSafeArea(.container, edges: .top)
            // Any scroll view that can scroll would otherwise turn the
            // title bar's material on over the header bar, for good.
            .withoutTopScrollEdge()
            .withWindow { window in
                window.titlebarAppearsTransparent = true
                window.titleVisibility = .hidden
                window.styleMask.insert(.fullSizeContentView)
                if window.toolbar == nil {
                    window.toolbar = NSToolbar(identifier: "header-bar")
                }
                window.toolbarStyle = .unifiedCompact
            }
    }
}

extension View {
    /// No scroll edge effect at the top of the scroll views within: macOS
    /// 26 draws it as the title bar's material, over the header bar.
    @ViewBuilder
    func withoutTopScrollEdge() -> some View {
        if #available(macOS 26, *) {
            scrollEdgeEffectHidden(true, for: .top)
        } else {
            self
        }
    }
}

/// The header bar: something in the middle, buttons at the end, and room at
/// the start for the window's own. The bar moves the window, and a double
/// click there does what it does on a title bar.
struct HeaderBar<Center: View, End: View>: View {
    @ViewBuilder var center: Center
    @ViewBuilder var end: End

    var body: some View {
        ZStack {
            WindowDragArea()
            center
            HStack(spacing: 6) {
                Spacer(minLength: 0)
                end
            }
            .padding(.trailing, 10)
        }
        .frame(height: Adw.headerHeight)
    }
}

/// Empty space that moves the window when dragged.
private struct WindowDragArea: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { DragView() }
    func updateNSView(_ view: NSView, context: Context) {}

    private final class DragView: NSView {
        override var mouseDownCanMoveWindow: Bool { true }

        override func mouseDown(with event: NSEvent) {
            guard let window else { return }
            if event.clickCount == 2 {
                // As the title bar's double click is set in System Settings.
                switch UserDefaults.standard.string(forKey: "AppleActionOnDoubleClick") {
                case "Minimize": window.performMiniaturize(nil)
                case "None": break
                default: window.performZoom(nil)
                }
            } else {
                window.performDrag(with: event)
            }
        }
    }
}

/// The header bar's `⚙` and `?`: a bold glyph in a 26 pt flat button, lit
/// while what it opens is showing.
struct HeaderGlyphButton: View {
    /// A character, or an SF Symbol's name with `symbol`.
    let glyph: String
    var symbol = false
    var lit = false
    let action: () -> Void
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        Button(action: action) {
            Group {
                if symbol {
                    Image(systemName: glyph).font(.system(size: 13, weight: .bold))
                } else {
                    Text(glyph).font(Adw.font(1, .bold))
                }
            }
            .foregroundStyle(Adw.foreground)
            .frame(width: 26, height: 26)
            .background(lit ? (colorScheme == .dark ? Color.white.opacity(0.2) : .white) : .clear,
                        in: RoundedRectangle(cornerRadius: Adw.radius))
        }
        .buttonStyle(AdwButtonStyle(padding: EdgeInsets()))
    }
}
