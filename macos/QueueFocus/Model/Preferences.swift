import Foundation

/// Settings only the Mac app has. They live in UserDefaults, never in
/// settings.json: the GNOME app rewrites that file whole, and would drop a
/// key it does not know.
enum Preferences {
    /// How wide the task title in the menu bar may grow, in points. macOS
    /// hides status items that do not fit, so a long title must not crowd
    /// this one out.
    static let menuBarTitleWidthKey = "menuBarTitleWidth"
    static let menuBarTitleWidthRange: ClosedRange<Double> = 100...600
    static let defaultMenuBarTitleWidth: Double = 200

    static func menuBarTitleWidth(in defaults: UserDefaults) -> Double {
        guard defaults.object(forKey: menuBarTitleWidthKey) != nil else {
            return defaultMenuBarTitleWidth
        }
        let width = defaults.double(forKey: menuBarTitleWidthKey)
        return min(max(width, menuBarTitleWidthRange.lowerBound), menuBarTitleWidthRange.upperBound)
    }
}
