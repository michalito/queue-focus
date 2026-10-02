import Testing
@testable import QueueFocus

/// The shortcuts' rules, on stand-in keys: the hosted tests never touch the
/// real shortcuts, which live in the user's defaults.
@Suite struct HotkeyTests {
    @Test func aKeyAnotherActionHasIsRefused() {
        let keys: [Hotkey: String] = [.toggleQueue: "⌃⌥Q", .quickAdd: "⌃⌥⇧Q", .showBoard: "⌃⌥B", .completeCurrent: "⌃⌥D"]
        #expect(Hotkey.holder(of: "⌃⌥B", besides: .toggleQueue, shortcuts: { keys[$0] }) == .showBoard)
        #expect(Hotkey.holder(of: "⌃⌥Q", besides: .toggleQueue, shortcuts: { keys[$0] }) == nil, "its own key is its own")
        #expect(Hotkey.holder(of: "⌃⌥X", besides: .toggleQueue, shortcuts: { keys[$0] }) == nil)
        #expect(Hotkey.holder(of: "⌃⌥X", besides: .toggleQueue, shortcuts: { _ in nil }) == nil, "cleared ones hold nothing")
    }
}
