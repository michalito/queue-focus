import XCTest

/// The Queue and Board windows: drags, drops and the task keys, on the real
/// windows over a data directory of the test's own.
@MainActor
final class WindowUITests: AppUITestCase {
    /// Ids follow the order: 1 now, 2 and 3 next, 4 side, 5 later.
    private let seed: [(String, String)] = [
        ("now task", "now"),
        ("first next", "next"),
        ("second next", "next"),
        ("a side", "side"),
        ("a later", "later"),
    ]

    private func element(_ id: String, in window: XCUIElement) -> XCUIElement {
        let element = window.descendants(matching: .any)[id].firstMatch
        XCTAssertTrue(element.waitForExistence(timeout: 5), "\(id) is in the window")
        return element
    }

    /// A task's title as shown: what a person picks the task up by. A row
    /// alone in its list is folded into the list by SwiftUI's accessibility,
    /// so the title is the dependable handle.
    private func title(_ text: String, in window: XCUIElement) -> XCUIElement {
        let title = window.staticTexts[text].firstMatch
        XCTAssertTrue(title.waitForExistence(timeout: 5), "\(text) is in the window")
        return title
    }

    /// Drag `source` and let go over `target`, at `dy` of its height.
    private func drag(_ source: XCUIElement, to target: XCUIElement, dy: CGFloat = 0.5) {
        let start = source.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: 0.5))
        let end = target.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: dy))
        // A mouse drag: `press(forDuration:thenDragTo:)` never starts one on macOS.
        start.click(forDuration: 0.6, thenDragTo: end, withVelocity: .slow, thenHoldForDuration: 0.3)
    }

    // MARK: Board

    func testABoardDropOnAHeadingLandsAtTheEndOfThatBucket() throws {
        try launch(with: seed)
        let board = openWindow("open-board", title: "Board")
        drag(title("a later", in: board), to: element("heading-next", in: board))
        waitForStore("Later's task joined the end of Next") {
            try self.stored("next") == ["first next", "second next", "a later"]
        }
    }

    func testABoardDropInFrontOfARowReordersTheBucket() throws {
        try launch(with: seed)
        let board = openWindow("open-board", title: "Board")
        drag(title("second next", in: board), to: element("task-2", in: board), dy: 0.15)
        waitForStore("second next moved in front of first next") {
            try self.stored("next") == ["second next", "first next"]
        }
    }

    func testABoardDropOnTheNowPanelMakesTheTaskCurrent() throws {
        try launch(with: seed)
        let board = openWindow("open-board", title: "Board")
        drag(title("a side", in: board), to: element("now-panel", in: board))
        waitForStore("the Side task is current and the old one leads Next") {
            try self.stored("now") == ["a side"] && self.stored("next") == ["now task", "first next", "second next"]
        }
    }

    func testTheCurrentTaskCanBeDraggedOutOfNow() throws {
        try launch(with: seed)
        let board = openWindow("open-board", title: "Board")
        drag(element("now-panel", in: board), to: element("heading-later", in: board))
        waitForStore("Now is empty and the task is at the end of Later") {
            try self.stored("now").isEmpty && self.stored("later") == ["a later", "now task"]
        }
        XCTAssertTrue(board.staticTexts["empty — drop a task here"].waitForExistence(timeout: 5))
    }

    // MARK: Queue

    func testAQueueDropOnTheBannerMakesTheTaskCurrent() throws {
        try launch(with: seed)
        let queue = openWindow("open-queue", title: "Queue")
        drag(title("first next", in: queue), to: element("now-panel", in: queue))
        waitForStore("first next is current") {
            try self.stored("now") == ["first next"]
        }
    }

    func testAQueueDropOnTheClosedLaterShelfLandsInLater() throws {
        try launch(with: seed)
        let queue = openWindow("open-queue", title: "Queue")
        drag(title("a side", in: queue), to: element("later-shelf", in: queue))
        waitForStore("the Side task went to the end of Later") {
            try self.stored("later") == ["a later", "a side"] && self.stored("side").isEmpty
        }
    }

    /// The task keys act on the focused task, from the moment the window opens.
    func testTheTaskKeysDriveTheQueue() throws {
        try launch(with: seed)
        let queue = openWindow("open-queue", title: "Queue")
        XCTAssertTrue(queue.isHittable)

        // Focus starts on the current task; j moves to Side's first task.
        app.typeKey("j", modifierFlags: [])
        app.typeKey("2", modifierFlags: [])
        waitForStore("2 moves the focused task to the end of Next") {
            try self.stored("next") == ["first next", "second next", "a side"]
        }
        app.typeKey("K", modifierFlags: .shift)
        waitForStore("K moves it up within Next") {
            try self.stored("next") == ["first next", "a side", "second next"]
        }
        app.typeKey("t", modifierFlags: [])
        waitForStore("t tags it") {
            let data = try Data(contentsOf: self.dataHome.appendingPathComponent("queue-focus/tasks.json"))
            return String(decoding: data, as: UTF8.self).contains("\"work\"")
        }
        app.typeKey("1", modifierFlags: [])
        waitForStore("1 makes it current") {
            try self.stored("now") == ["a side"]
        }
        app.typeKey("d", modifierFlags: [])
        waitForStore("d completes it and the task it replaced comes back") {
            try self.stored("now") == ["now task"]
        }
        app.typeKey("r", modifierFlags: [])
        let field = queue.textFields["rename-field"]
        XCTAssertTrue(field.waitForExistence(timeout: 5), "r renames in place")
        field.typeText("renamed\r")
        waitForStore("the rename is saved") {
            try self.stored("now") == ["renamed"]
        }
        app.typeKey("l", modifierFlags: [])
        XCTAssertTrue(queue.staticTexts["a later"].waitForExistence(timeout: 5), "l opens the Later shelf")
        app.typeKey(.escape, modifierFlags: [])
        let closed = NSPredicate { _, _ in MainActor.assumeIsolated { !queue.exists } }
        XCTAssertEqual(XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: closed, object: nil)], timeout: 5), .completed,
                       "Escape closes the window")
    }

    func testTheWindowAddFieldAddsWhereSettingsSay() throws {
        try launch(with: seed)
        let queue = openWindow("open-queue", title: "Queue")
        app.typeKey("n", modifierFlags: [])
        queue.textFields["window-add-field"].typeText("from the window\r")
        waitForStore("Return adds to Next") {
            try self.stored("next").last == "from the window"
        }
        queue.textFields["window-add-field"].typeText("right now")
        queue.textFields["window-add-field"].typeKey(.return, modifierFlags: .command)
        waitForStore("Command-Return adds as current") {
            try self.stored("now") == ["right now"]
        }
    }
}
