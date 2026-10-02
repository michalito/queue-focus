import AppKit
import SwiftUI

extension Color {
    /// Grey text that still reads: 4.5 to 1 or more on the windows' and the
    /// forms' backgrounds in either appearance, which the system's secondary
    /// grey is not. For lines that explain a setting, and the Board's Later
    /// titles.
    static let readableGrey = Color(nsColor: NSColor(name: "readableGrey") { appearance in
        appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            ? NSColor(white: 0.68, alpha: 1)
            : NSColor(white: 0.38, alpha: 1)
    })
}
