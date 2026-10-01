import CoreGraphics
import Testing
@testable import QueueFocus

private func task(_ id: UInt64, _ bucket: Bucket) -> QueueTask {
    QueueTask(id: id, title: "t\(id)", bucket: bucket, tag: nil, createdAt: 0,
              startedAt: bucket == .now ? 0 : nil, pausedAt: nil)
}

private func snapshot(current: UInt64? = 4, side: [UInt64] = [3], next: [UInt64] = [1], later: [UInt64] = [2]) -> QueueSnapshot {
    QueueSnapshot(
        current: current.map { task($0, .now) },
        side: side.map { task($0, .side) },
        next: next.map { task($0, .next) },
        later: later.map { task($0, .later) },
        revision: 0
    )
}

/// Ported from the GTK app's placement tests.
@Suite struct PlacementTests {
    @Test func bothPagesReadCurrentSideNextLater() {
        let s = snapshot()
        #expect(Placement.visibleOrder(s, page: .queue, laterOpen: false) == [4, 3, 1])
        #expect(Placement.visibleOrder(s, page: .queue, laterOpen: true) == [4, 3, 1, 2])
        // Later has a column of its own on the Board: never collapsed.
        #expect(Placement.visibleOrder(s, page: .board, laterOpen: false) == [4, 3, 1, 2])
        // An empty Now leaves the lists as they were.
        #expect(Placement.visibleOrder(snapshot(current: nil, side: [], later: []), page: .board, laterOpen: false) == [1])
    }

    @Test func focusComesBackToTheSameTaskOrToWhereItWas() throws {
        let old: [UInt64] = [1, 2, 3, 4]
        let three = try #require(FocusMemory(order: old, id: 3))
        #expect(three.restore(in: [3, 1, 2, 4]) == 3)
        #expect(three.restore(in: [1, 2, 4]) == 4)
        #expect(try #require(FocusMemory(order: old, id: 4)).restore(in: [1, 2]) == 2)
        #expect(try #require(FocusMemory(order: old, id: 1)).restore(in: []) == nil)
        #expect(FocusMemory(order: old, id: 99) == nil)
    }

    @Test func jAndKStepThroughTheOrderAndStopAtTheEnds() {
        let order: [UInt64] = [4, 3, 1]
        #expect(Placement.step(from: 4, by: 1, in: order) == 3)
        #expect(Placement.step(from: 1, by: 1, in: order) == 1)
        #expect(Placement.step(from: 4, by: -1, in: order) == 4)
        #expect(Placement.step(from: nil, by: 1, in: order) == 4)
        #expect(Placement.step(from: nil, by: -1, in: order) == 1)
        #expect(Placement.step(from: 99, by: 1, in: order) == 4)
        #expect(Placement.step(from: 1, by: 1, in: []) == nil)
    }

    /// GTK's `anchor_at`: the top half of a row is in front of it, the bottom
    /// half in front of the next, and past the last row's middle is the end.
    @Test func aDropLandsInFrontOfTheRowItIsOverOrTheNextOne() {
        let rows: [(id: UInt64, frame: CGRect)] = [
            (10, CGRect(x: 0, y: 0, width: 100, height: 20)),
            (11, CGRect(x: 0, y: 24, width: 100, height: 20)),
            (12, CGRect(x: 0, y: 48, width: 100, height: 20)),
        ]
        #expect(Placement.anchor(at: 0, rows: rows) == 10)
        #expect(Placement.anchor(at: 9, rows: rows) == 10)
        #expect(Placement.anchor(at: 11, rows: rows) == 11)
        #expect(Placement.anchor(at: 22, rows: rows) == 11, "the gap between rows")
        #expect(Placement.anchor(at: 57, rows: rows) == 12)
        #expect(Placement.anchor(at: 59, rows: rows) == nil, "past the last middle is the end")
        #expect(Placement.anchor(at: 400, rows: rows) == nil)
        #expect(Placement.anchor(at: 5, rows: []) == nil)
    }
}

@Suite struct KeyMapTests {
    private func key(_ characters: String, shift: Bool = false, capsLock: Bool = false) -> Keystroke {
        Keystroke(characters: characters, shift: shift, capsLock: capsLock)
    }

    @Test func theTaskKeysActOnTheFocusedTask() {
        let id: UInt64 = 7
        let expected: [(Keystroke, KeyAction)] = [
            (key("J", shift: true), .shift(id, 1)),
            (key("K", shift: true), .shift(id, -1)),
            (key("d"), .complete(id)),
            (key("x"), .complete(id)),
            (Keystroke(special: .delete), .complete(id)),
            (Keystroke(special: .deleteForward), .complete(id)),
            (Keystroke(special: .return), .promote(id)),
            (key("t"), .cycleTag(id)),
            (key("1"), .promote(id)),
            (key("2"), .move(id, .next)),
            (key("3"), .move(id, .later)),
            (key("4"), .move(id, .side)),
            (key("r"), .rename(id)),
            (Keystroke(special: .f2), .rename(id)),
        ]
        for (stroke, action) in expected {
            #expect(KeyMap.action(for: stroke, focused: id) == action, "\(stroke)")
            #expect(KeyMap.action(for: stroke, focused: nil) == nil, "\(stroke) needs a task")
        }
    }

    @Test func theWindowKeysNeedNoTask() {
        let expected: [(Keystroke, KeyAction)] = [
            (Keystroke(special: .escape), .close),
            (key("n"), .focusAdd),
            (key("/"), .focusAdd),
            (key("a"), .focusAdd),
            (key("b"), .show(.board)),
            (key("q"), .show(.queue)),
            (key("l"), .toggleLater),
            (key("j"), .focusStep(1)),
            (key("k"), .focusStep(-1)),
            (key("p"), .togglePause),
            (key("?", shift: true), .toggleShortcuts),
        ]
        for (stroke, action) in expected {
            #expect(KeyMap.action(for: stroke, focused: nil) == action, "\(stroke)")
        }
    }

    @Test func capsLockIsNotShift() {
        #expect(KeyMap.action(for: key("J", capsLock: true), focused: 1) == .focusStep(1))
        #expect(KeyMap.action(for: key("J", shift: true, capsLock: true), focused: 1) == .shift(1, 1))
    }

    @Test func menuKeysAreLeftToTheMenus() {
        var command = key("q")
        command.commandLike = true
        #expect(KeyMap.action(for: command, focused: 1) == nil)
        #expect(KeyMap.action(for: key("z"), focused: 1) == nil)
    }
}
