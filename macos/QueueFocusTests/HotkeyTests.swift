import Carbon.HIToolbox
import Foundation
import Testing
@testable import QueueFocus

/// The four modifier keys, without the Fn and Num Lock bits the system's own
/// list sometimes carries.
private let modifierBits = cmdKey | shiftKey | optionKey | controlKey

/// The shortcuts this Mac's system has turned on (Mission Control, Spotlight,
/// screenshots, input sources…), as System Settings › Keyboard lists them.
/// On the main actor: Carbon's reader is not safe to call from two threads.
@MainActor
private func systemShortcuts() -> [KeyCombo]? {
    var list: Unmanaged<CFArray>?
    guard CopySymbolicHotKeys(&list) == noErr, let entries = list?.takeRetainedValue() as? [[String: Any]] else {
        return nil
    }
    return entries.compactMap { entry in
        guard entry[kHISymbolicHotKeyEnabled as String] as? Bool == true,
              let code = entry[kHISymbolicHotKeyCode as String] as? Int,
              let modifiers = entry[kHISymbolicHotKeyModifiers as String] as? Int
        else { return nil }
        return KeyCombo(carbonKeyCode: code, carbonModifiers: modifiers & modifierBits)
    }
}

/// The letter keys, by Carbon's numbering, which is not alphabetical.
private let letterKeys: Set<Int> = [
    kVK_ANSI_A, kVK_ANSI_B, kVK_ANSI_C, kVK_ANSI_D, kVK_ANSI_E, kVK_ANSI_F, kVK_ANSI_G, kVK_ANSI_H, kVK_ANSI_I,
    kVK_ANSI_J, kVK_ANSI_K, kVK_ANSI_L, kVK_ANSI_M, kVK_ANSI_N, kVK_ANSI_O, kVK_ANSI_P, kVK_ANSI_Q, kVK_ANSI_R,
    kVK_ANSI_S, kVK_ANSI_T, kVK_ANSI_U, kVK_ANSI_V, kVK_ANSI_W, kVK_ANSI_X, kVK_ANSI_Y, kVK_ANSI_Z,
]

/// Shortcuts in `keys` the system has taken on this Mac. A clash fails on a
/// build machine, which is a clean Mac; here it is reported, since the user
/// may have bound the key on purpose.
@MainActor
private func expectNoneTaken(_ keys: [(String, KeyCombo)]) throws {
    let taken = Set(try #require(systemShortcuts(), "the system's shortcuts can be read"))
    let clashes = keys.filter { taken.contains($0.1) }.map(\.0)
    guard !clashes.isEmpty else { return }
    let message = Comment(rawValue: "taken by the system on this Mac: \(clashes.joined(separator: ", "))")
    if ProcessInfo.processInfo.environment["CI"] != nil {
        Issue.record(message)
    } else {
        withKnownIssue(message) { Issue.record(message) }
    }
}

/// The shortcuts' rules, on stand-in keys: the hosted tests never touch the
/// real shortcuts, which live in the user's defaults.
@MainActor
@Suite struct HotkeyTests {
    @Test func aKeyAnotherActionHasIsRefused() {
        let keys: [Hotkey: String] = [.toggleQueue: "⌃⌥Q", .quickAdd: "⌃⌥⇧Q", .showBoard: "⌃⌥B", .completeCurrent: "⌃⌥D"]
        #expect(Hotkey.holder(of: "⌃⌥B", besides: .toggleQueue, shortcuts: { keys[$0] }) == .showBoard)
        #expect(Hotkey.holder(of: "⌃⌥Q", besides: .toggleQueue, shortcuts: { keys[$0] }) == nil, "its own key is its own")
        #expect(Hotkey.holder(of: "⌃⌥X", besides: .toggleQueue, shortcuts: { keys[$0] }) == nil)
        #expect(Hotkey.holder(of: "⌃⌥X", besides: .toggleQueue, shortcuts: { _ in nil }) == nil, "cleared ones hold nothing")
    }

    @Test func theDefaultsAreControlOptionWithALetterAndAllDifferent() {
        let defaults = Hotkey.allCases.map(\.defaultKeys)
        #expect(Set(defaults).count == defaults.count, "four keys for four actions")
        for keys in defaults {
            #expect(letterKeys.contains(keys.carbonKeyCode))
            #expect(keys.carbonModifiers & (controlKey | optionKey) == controlKey | optionKey)
            #expect(keys.carbonModifiers & cmdKey == 0, "Command belongs to the apps' menus")
        }
        #expect(Set(defaults).isDisjoint(with: MenuKey.allCases.map(\.keys)))
    }

    @Test func noDefaultIsOneOfTheSystemsShortcuts() throws {
        try expectNoneTaken(Hotkey.allCases.map { (String(describing: $0), $0.defaultKeys) })
    }

    @Test func noMenuKeyIsOneOfTheSystemsShortcuts() throws {
        let settings = KeyCombo(carbonKeyCode: kVK_ANSI_Comma, carbonModifiers: cmdKey)
        try expectNoneTaken(MenuKey.allCases.map { ("⌘\($0.character)", $0.keys) } + [("⌘,", settings)])
    }
}

