import KeyboardShortcuts

/// The global shortcuts, which work whatever app is in front. They are
/// Carbon hot keys, through KeyboardShortcuts, so they need no Accessibility
/// permission. Each can be changed in Settings, which stores it in the app's
/// defaults.
enum Hotkey: CaseIterable {
    case toggleQueue
    case quickAdd
    case showBoard
    case completeCurrent

    var name: KeyboardShortcuts.Name {
        switch self {
        case .toggleQueue: .toggleQueue
        case .quickAdd: .quickAdd
        case .showBoard: .showBoard
        case .completeCurrent: .completeCurrent
        }
    }

    /// What it does, as Settings lists it.
    var title: String {
        switch self {
        case .toggleQueue: "Show or hide the Queue"
        case .quickAdd: "Quick add"
        case .showBoard: "Show the Board"
        case .completeCurrent: "Complete the current task"
        }
    }

    /// Have `perform` told whenever one is pressed. Once, at launch.
    @MainActor
    static func install(_ perform: @escaping @MainActor (Hotkey) -> Void) {
        for hotkey in allCases {
            KeyboardShortcuts.onKeyDown(for: hotkey.name) { perform(hotkey) }
        }
    }
}

extension KeyboardShortcuts.Name {
    // The defaults chosen for the Mac: Control-Option with the GNOME letters.
    static let toggleQueue = Self("toggleQueue", initial: .init(.q, modifiers: [.control, .option]))
    static let quickAdd = Self("quickAdd", initial: .init(.q, modifiers: [.control, .option, .shift]))
    static let showBoard = Self("showBoard", initial: .init(.b, modifiers: [.control, .option]))
    static let completeCurrent = Self("completeCurrent", initial: .init(.d, modifiers: [.control, .option]))
}
