import Foundation

/// A flash on demand, to see a style without waiting for the reminder:
/// `-flashPreview edges`, with `-flashIntensity strong` and
/// `-flashPalette orange` if wanted, or `-flashPreview all` for the six in
/// turn. The app starts as always; two seconds in, the flash is drawn with a
/// sample task, apart from the engine and the queue.
enum FlashPreview {
    /// Seconds between the styles of `all`.
    static let spacing: Double = 2.5

    static func events(_ defaults: UserDefaults) -> [FlashEvent] {
        guard let name = defaults.string(forKey: "flashPreview") else { return [] }
        let intensity = defaults.string(forKey: "flashIntensity").flatMap(intensity) ?? .normal
        let palette = defaults.string(forKey: "flashPalette").flatMap(palette) ?? .blue
        let styles: [FlashStyle] = name == "all" ? allStyles : style(name).map { [$0] } ?? []
        return styles.map { style in
            FlashEvent(style: style, intensity: intensity, palette: palette, title: "Write the quarterly report", timer: "23m")
        }
    }

    @MainActor
    static func schedule(_ events: [FlashEvent], on flash: FlashController) {
        for (index, event) in events.enumerated() {
            DispatchQueue.main.asyncAfter(deadline: .now() + 2 + Double(index) * spacing) { [weak flash] in
                MainActor.assumeIsolated { flash?.show(event) }
            }
        }
    }

    static let allStyles: [FlashStyle] = [.wash, .wash2, .edges, .edgesSoft, .topbar, .topbarBeam]

    /// The engine's names for them, as in settings.json.
    static func style(_ name: String) -> FlashStyle? {
        switch name {
        case "wash": .wash
        case "wash2": .wash2
        case "edges": .edges
        case "edgesSoft": .edgesSoft
        case "topbar": .topbar
        case "topbarBeam": .topbarBeam
        default: nil
        }
    }

    static func intensity(_ name: String) -> Intensity? {
        switch name {
        case "subtle": .subtle
        case "normal": .normal
        case "strong": .strong
        default: nil
        }
    }

    static func palette(_ name: String) -> Palette? {
        switch name {
        case "blue": .blue
        case "orange": .orange
        default: nil
        }
    }
}
