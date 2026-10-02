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
    let tag: TaskTag

    var body: some View {
        Text(TagStyle.letter(tag))
            .font(.caption2.weight(.bold))
            .foregroundStyle(.white)
            .padding(.horizontal, 5)
            .padding(.vertical, 1)
            .background(TagStyle.accent(tag), in: RoundedRectangle(cornerRadius: 4))
            .accessibilityLabel(Text(TagStyle.name(tag)))
    }
}
