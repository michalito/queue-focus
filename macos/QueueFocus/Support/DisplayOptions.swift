import AppKit
import Observation
import SwiftUI

/// The system's accessibility display settings that the app's look follows.
struct DisplayOptions: Equatable {
    var reduceMotion = false
    var increaseContrast = false
    var reduceTransparency = false
    var differentiateWithoutColor = false

    /// The settings named in `-displayOptions increaseContrast,reduceMotion`:
    /// what the UI tests and a check by eye use, without touching the
    /// user's System Settings.
    init(names: String) {
        let names = Set(names.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) })
        reduceMotion = names.contains("reduceMotion")
        increaseContrast = names.contains("increaseContrast")
        reduceTransparency = names.contains("reduceTransparency")
        differentiateWithoutColor = names.contains("differentiateWithoutColor")
    }

    init(reduceMotion: Bool = false, increaseContrast: Bool = false, reduceTransparency: Bool = false,
         differentiateWithoutColor: Bool = false) {
        self.reduceMotion = reduceMotion
        self.increaseContrast = increaseContrast
        self.reduceTransparency = reduceTransparency
        self.differentiateWithoutColor = differentiateWithoutColor
    }

    /// What the system says, unless the launch arguments say otherwise.
    @MainActor
    static func current(_ defaults: UserDefaults = .standard) -> DisplayOptions {
        if let names = defaults.string(forKey: "displayOptions") {
            return DisplayOptions(names: names)
        }
        let workspace = NSWorkspace.shared
        return DisplayOptions(
            reduceMotion: workspace.accessibilityDisplayShouldReduceMotion,
            increaseContrast: workspace.accessibilityDisplayShouldIncreaseContrast,
            reduceTransparency: workspace.accessibilityDisplayShouldReduceTransparency,
            differentiateWithoutColor: workspace.accessibilityDisplayShouldDifferentiateWithoutColor
        )
    }
}

/// The display options now, kept up to date as the user changes them.
@MainActor
@Observable
final class Display {
    private(set) var options: DisplayOptions
    /// Told after a change, for what SwiftUI does not draw: the status item.
    @ObservationIgnored var didChange: @MainActor () -> Void = {}
    @ObservationIgnored private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        options = .current(defaults)
        NSWorkspace.shared.notificationCenter.addObserver(
            self, selector: #selector(optionsDidChange),
            name: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification, object: nil
        )
    }

    @objc private func optionsDidChange(_ notification: Notification) {
        let options = DisplayOptions.current(defaults)
        guard options != self.options else { return }
        self.options = options
        didChange()
    }
}

extension EnvironmentValues {
    @Entry var displayOptions = DisplayOptions()
}

/// `content` with the display options in its environment, and again
/// whenever they change.
struct FollowingDisplay<Content: View>: View {
    let display: Display
    @ViewBuilder let content: () -> Content

    var body: some View {
        content().environment(\.displayOptions, display.options)
    }
}
