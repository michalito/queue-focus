import SwiftUI

/// Where tasks sit in the windows, and how the keyboard and drops move among
/// them. Pure values, ported from the GTK app's `placement.rs`, so the rules
/// are the same on both platforms and testable without a window.

/// The two task windows.
enum Page: Equatable {
    case queue
    case board
}

enum Placement {
    /// The order focus moves through: the current task, then Side, Next and
    /// Later. The Queue shows Later only while its shelf is open; the Board
    /// gives Later a column of its own, so it is always there.
    static func visibleOrder(_ snapshot: QueueSnapshot, page: Page, laterOpen: Bool) -> [UInt64] {
        var order: [UInt64] = []
        if let current = snapshot.current {
            order.append(current.id)
        }
        order += snapshot.side.map(\.id)
        order += snapshot.next.map(\.id)
        if laterOpen || page == .board {
            order += snapshot.later.map(\.id)
        }
        return order
    }

    /// The task one step from `id` in `order`, stopping at either end. With
    /// nothing focused, `j` starts at the top and `k` at the bottom.
    static func step(from id: UInt64?, by delta: Int, in order: [UInt64]) -> UInt64? {
        guard !order.isEmpty else { return nil }
        guard let id, let index = order.firstIndex(of: id) else {
            return delta > 0 ? order.first : order.last
        }
        return order[min(max(index + delta, 0), order.count - 1)]
    }

    /// Where a drop at `y` lands in a list whose rows run top to bottom: in
    /// front of the row whose top half it is over, in front of the next row
    /// from a row's bottom half, and at the end (`nil`) past the last row's
    /// middle.
    static func anchor(at y: CGFloat, rows: [(id: UInt64, frame: CGRect)]) -> UInt64? {
        rows.first { y < $0.frame.midY }?.id
    }
}

/// The focused task, remembered by id and position, so focus can come back
/// after the list changes: to the same task if it is still shown, otherwise
/// to whatever now sits where it was.
struct FocusMemory: Equatable {
    let id: UInt64
    let index: Int

    init?(order: [UInt64], id: UInt64) {
        guard let index = order.firstIndex(of: id) else { return nil }
        self.id = id
        self.index = index
    }

    func restore(in order: [UInt64]) -> UInt64? {
        if order.contains(id) {
            return id
        }
        guard !order.isEmpty else { return nil }
        return order[min(index, order.count - 1)]
    }
}

// MARK: Keys

/// A key press, as much of it as the task keys read.
struct Keystroke: Equatable {
    enum Special: Equatable {
        case escape, `return`, delete, deleteForward, f2
    }

    var characters: String = ""
    var special: Special?
    var shift = false
    var capsLock = false
    /// Command, Control or Option: those keys belong to the menus.
    var commandLike = false

    /// What was typed, with Caps Lock undone: `J` means Shift-J, never a
    /// lowercase j typed with Caps Lock on.
    var typed: String {
        capsLock && !shift ? characters.lowercased() : characters
    }
}

/// What a key asks a task window to do.
enum KeyAction: Equatable {
    case close
    case focusAdd
    case show(Page)
    case toggleLater
    case focusStep(Int)
    case togglePause
    case toggleShortcuts
    case rename(UInt64)
    case shift(UInt64, Int)
    case complete(UInt64)
    case cycleTag(UInt64)
    case promote(UInt64)
    case move(UInt64, Bucket)
}

/// The task keys, as the GTK app has them. They act on the focused task.
enum KeyMap {
    static func action(for key: Keystroke, focused: UInt64?) -> KeyAction? {
        if key.commandLike { return nil }
        switch key.special {
        case .escape: return .close
        case .return: return focused.map(KeyAction.promote)
        case .delete, .deleteForward: return focused.map(KeyAction.complete)
        case .f2: return focused.map(KeyAction.rename)
        case nil: break
        }
        switch key.typed {
        case "n", "/", "a": return .focusAdd
        case "b": return .show(.board)
        case "q": return .show(.queue)
        case "l": return .toggleLater
        case "j": return .focusStep(1)
        case "k": return .focusStep(-1)
        case "p": return .togglePause
        case "?": return .toggleShortcuts
        default: break
        }
        guard let id = focused else { return nil }
        switch key.typed {
        case "r": return .rename(id)
        case "J": return .shift(id, 1)
        case "K": return .shift(id, -1)
        case "d", "x": return .complete(id)
        case "t": return .cycleTag(id)
        case "1": return .promote(id)
        case "2": return .move(id, .next)
        case "3": return .move(id, .later)
        case "4": return .move(id, .side)
        default: return nil
        }
    }
}

extension Keystroke {
    /// Read a SwiftUI key press. Letters and punctuation come by what was
    /// typed; the keys that type nothing come by their key.
    init(_ press: KeyPress) {
        let special: Special? = switch press.key {
        case .escape: .escape
        case .return: .return
        case .delete: .delete
        case .deleteForward: .deleteForward
        case KeyEquivalent(Character(UnicodeScalar(NSF2FunctionKey)!)): .f2
        default: nil
        }
        self.init(
            characters: press.characters,
            special: special,
            shift: press.modifiers.contains(.shift),
            capsLock: press.modifiers.contains(.capsLock),
            commandLike: !press.modifiers.isDisjoint(with: [.command, .control, .option])
        )
    }
}

/// The `?` popover's list, as GTK's, with the Mac's own keys.
let shortcutList: [(keys: String, does: String)] = [
    ("j / k", "move"),
    ("J / K", "reorder"),
    ("⏎", "make current"),
    ("d", "done"),
    ("p", "pause"),
    ("1–4", "Now, Next, Later, Side"),
    ("t", "tag"),
    ("r", "rename"),
    ("l", "Later shelf"),
    ("n", "add"),
    ("q / b", "Queue / Board"),
    ("⌘,", "settings"),
]
