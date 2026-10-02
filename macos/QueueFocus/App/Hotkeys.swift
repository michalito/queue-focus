import Carbon.HIToolbox
import KeyboardShortcuts

/// A key and its modifiers, as Carbon numbers them: what a hot key is.
struct KeyCombo: Hashable {
    let carbonKeyCode: Int
    let carbonModifiers: Int
}

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

    /// The default chosen for the Mac: Control-Option with GNOME's letters
    /// (Super is the Mac's Command, which belongs to the apps' menus).
    var defaultKeys: KeyCombo {
        switch self {
        case .toggleQueue: KeyCombo(carbonKeyCode: kVK_ANSI_Q, carbonModifiers: controlKey | optionKey)
        case .quickAdd: KeyCombo(carbonKeyCode: kVK_ANSI_Q, carbonModifiers: controlKey | optionKey | shiftKey)
        case .showBoard: KeyCombo(carbonKeyCode: kVK_ANSI_B, carbonModifiers: controlKey | optionKey)
        case .completeCurrent: KeyCombo(carbonKeyCode: kVK_ANSI_D, carbonModifiers: controlKey | optionKey)
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

    /// Another of the four that already has `shortcut`: a recorder refuses
    /// it, since two actions on one key would both run.
    static func holder<Shortcut: Equatable>(of shortcut: Shortcut, besides hotkey: Hotkey,
                                            shortcuts: (Hotkey) -> Shortcut?) -> Hotkey? {
        allCases.first { $0 != hotkey && shortcuts($0) == shortcut }
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
    static let toggleQueue = Self("toggleQueue", initial: .init(Hotkey.toggleQueue.defaultKeys))
    static let quickAdd = Self("quickAdd", initial: .init(Hotkey.quickAdd.defaultKeys))
    static let showBoard = Self("showBoard", initial: .init(Hotkey.showBoard.defaultKeys))
    static let completeCurrent = Self("completeCurrent", initial: .init(Hotkey.completeCurrent.defaultKeys))
}

private extension KeyboardShortcuts.Shortcut {
    init(_ keys: KeyCombo) {
        self.init(carbonKeyCode: keys.carbonKeyCode, carbonModifiers: keys.carbonModifiers)
    }
}

/// The app's own Command keys, as its View menu has them and as Carbon
/// numbers them, so the tests can look for them among the system's.
enum MenuKey: CaseIterable {
    case queue
    case board
    case quickAdd

    var character: Character {
        switch self {
        case .queue: "1"
        case .board: "2"
        case .quickAdd: "n"
        }
    }

    var keys: KeyCombo {
        let code = switch self {
        case .queue: kVK_ANSI_1
        case .board: kVK_ANSI_2
        case .quickAdd: kVK_ANSI_N
        }
        return KeyCombo(carbonKeyCode: code, carbonModifiers: cmdKey)
    }
}
