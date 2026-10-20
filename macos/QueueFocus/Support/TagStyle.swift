import AppKit
import SwiftUI

/// The tag colours, as GNOME draws them: blue for work, orange for personal.
enum TagStyle {
    /// The accent a tag lends the Now card, its chip and the Done button.
    static func accent(_ tag: TaskTag?) -> Color {
        switch tag {
        case .work: Color(red: 0x35 / 255, green: 0x84 / 255, blue: 0xE4 / 255)
        case .personal: Color(red: 0xC6 / 255, green: 0x46 / 255, blue: 0x00 / 255)
        case nil: .secondary
        }
    }

    /// The colour behind white text: the chip, and controls tinted by the
    /// tag. Not GNOME's accent: white on that blue is 3.8 to 1, under the
    /// 4.5 small text needs, so GNOME's next blue down (4.8); darker still
    /// with Increase Contrast. Red, green and blue out of 255.
    static func solidRGB(_ tag: TaskTag, increaseContrast: Bool) -> (Double, Double, Double) {
        switch (tag, increaseContrast) {
        case (.work, false): (0x1C, 0x71, 0xD8)
        case (.work, true): (0x1A, 0x5F, 0xB4)
        case (.personal, false): (0xC6, 0x46, 0x00)
        case (.personal, true): (0xA1, 0x35, 0x00)
        }
    }

    static func solid(_ tag: TaskTag?, _ options: DisplayOptions) -> Color {
        guard let tag else { return .accentColor }
        let (red, green, blue) = solidRGB(tag, increaseContrast: options.increaseContrast)
        return Color(red: red / 255, green: green / 255, blue: blue / 255)
    }

    /// The dot in the menu bar: brighter, so it reads on either menu bar.
    static func dot(_ tag: TaskTag?) -> NSColor {
        switch tag {
        case .work: NSColor(srgbRed: 0x62 / 255, green: 0xA0 / 255, blue: 0xEA / 255, alpha: 1)
        case .personal: NSColor(srgbRed: 0xFF / 255, green: 0xA3 / 255, blue: 0x48 / 255, alpha: 1)
        case nil: .tertiaryLabelColor
        }
    }

    /// `W` or `P`, the chip's letter.
    static func letter(_ tag: TaskTag) -> String {
        switch tag {
        case .work: "W"
        case .personal: "P"
        }
    }

    static func name(_ tag: TaskTag) -> String {
        switch tag {
        case .work: "work"
        case .personal: "personal"
        }
    }
}

/// A task's tag as a small coloured letter.
struct TagChip: View {
    @Environment(\.displayOptions) private var options
    let tag: TaskTag

    var body: some View {
        Text(TagStyle.letter(tag))
            .font(.caption2.weight(.bold))
            .foregroundStyle(.white)
            .padding(.horizontal, 5)
            .padding(.vertical, 1)
            .background(TagStyle.solid(tag, options), in: RoundedRectangle(cornerRadius: 4))
            .accessibilityLabel(Text(TagStyle.name(tag)))
    }
}
