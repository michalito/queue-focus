import XCTest

/// What VoiceOver finds: every row and control there, under its name.
@MainActor
final class AccessibilityUITests: AppUITestCase {
    private func element(_ id: String, in parent: XCUIElement) -> XCUIElement {
        parent.descendants(matching: .any)[id].firstMatch
    }

    private let seed: [(String, String)] = [("ship v0.1 #w", "now"), ("write notes #p", "next"), ("a side", "side"), ("a later", "later")]

    /// XCTest's audit of what is on screen, failing on the two kinds of issue
    /// worth failing on: text too faint to read, and controls VoiceOver
    /// cannot tell apart. SwiftUI's own noise is let through: the hosting
    /// view's nameless root group, the Touch Bar. Every issue goes to the
    /// log, so a new one is seen, and the test fails on its own account: an
    /// audit that fails attaches pictures of the screen, which the tests
    /// never keep.
    private func audit(_ surface: String, file: StaticString = #filePath, line: UInt = #line) throws {
        var failures: [String] = []
        let touchBar = app.touchBars.firstMatch
        let touchBarFrame = touchBar.exists ? touchBar.frame.insetBy(dx: -4, dy: -4) : .null
        let windows = app.windows.allElementsBoundByIndex.map { (frame: $0.frame, title: $0.title) }
        try app.performAccessibilityAudit(for: [.contrast, .sufficientElementDescription]) { issue in
            let element = issue.element
            // The Touch Bar's items are the system's, for the text field.
            let inTouchBar = element.map { touchBarFrame.contains($0.frame) } ?? false
            // Contrast is measured from the screen: text scrolled out of its
            // window is measured against whatever is there instead, and a
            // window's title is the system's.
            let unseen = element.map { element in
                !windows.contains { $0.frame.contains(element.frame) }
                    || windows.contains { $0.title == element.value as? String }
            } ?? false
            // A window with no title is the system's, such as a text field's
            // completions; a slider's thumb is part of the slider, which has
            // the name.
            let ignored = issue.auditType == .sufficientElementDescription
                && (inTouchBar || element?.elementType == .touchBar
                    || (element?.elementType == .group && element?.identifier.isEmpty == true)
                    || (element?.elementType == .window && element?.title.isEmpty == true)
                    || element?.elementType == .valueIndicator)
                || issue.auditType == .contrast
                && (unseen || ["setting-note", "flash-status"].contains(element?.identifier ?? ""))
            let what = element.map { $0.debugDescription.split(separator: "\n").prefix(3).joined(separator: " / ") } ?? "no element"
            print("AUDIT \(surface): \(ignored ? "ignored" : "FAILED"): \(issue.compactDescription) — \(what)")
            if !ignored {
                failures.append("\(issue.compactDescription): \(what)")
            }
            return true
        }
        if !TestEnvironment.isAuditBaseline {
            XCTExpectFailure("the audit's findings are macOS 27's", options: .nonStrict())
        }
        XCTAssertTrue(failures.isEmpty, failures.joined(separator: "\n"), file: file, line: line)
    }

    func testThePopoverPassesTheAudit() throws {
        try launch(with: seed)
        _ = openPopover()
        try audit("popover")
    }

    func testTheQueuePassesTheAudit() throws {
        try launch(with: seed)
        _ = openWindow("open-queue", title: "Queue")
        app.typeKey("l", modifierFlags: [])
        try audit("queue")
    }

    func testTheBoardPassesTheAudit() throws {
        try launch(with: seed)
        _ = openWindow("open-board", title: "Board")
        try audit("board")
    }

    func testSettingsPassesTheAudit() throws {
        try launch(with: seed)
        _ = openPopover()
        click(app.menuButtons["gear-menu"])
        click(app.menuItems["gear-settings"])
        XCTAssertTrue(app.windows["com_apple_SwiftUI_Settings_window"].waitForExistence(timeout: 5))
        try audit("settings")
    }

    /// With the system asked for more contrast, the colours change to pass it.
    func testWithMoreContrastTheWindowsPassTheAudit() throws {
        try launch(with: seed, arguments: ["-displayOptions", "increaseContrast,differentiateWithoutColor"])
        _ = openWindow("open-board", title: "Board")
        try audit("board, more contrast")
        XCTAssertTrue(statusTitle.contains("W"), "the menu bar names the tag, not only its colour: \(statusTitle)")
    }

    /// The audit takes an SF Symbol's own name ("Ellipsis") as a label, so
    /// every icon-only control's label is checked here.
    func testIconOnlyControlsSayWhatTheyDo() throws {
        try launch(with: seed)
        let queue = openWindow("open-queue", title: "Queue")
        let labels = [
            "promote-2": "Make current",
            "now-done": "Done",
            "shortcuts": "Keyboard shortcuts",
        ]
        for (id, label) in labels {
            let control = element(id, in: queue)
            XCTAssertTrue(control.waitForExistence(timeout: 5), id)
            XCTAssertEqual(control.label, label, id)
        }
        // VoiceOver reads a menu button by its title.
        XCTAssertEqual(element("menu-2", in: queue).title, "Task menu")
        XCTAssertTrue(element("now-timer", in: queue).label.hasPrefix("Pause, "))
        XCTAssertEqual(element("later-shelf", in: queue).label, "Later, 1")
        XCTAssertEqual(queue.buttons["Settings"].firstMatch.label, "Settings")
    }

    /// SwiftUI drops a group's only element, which once took a bucket's only
    /// task with it.
    func testABucketWithOneTaskKeepsItsRow() throws {
        try launch(with: [("only next", "next"), ("only side", "side")])
        let queue = openWindow("open-queue", title: "Queue")
        let next = element("list-next", in: queue)
        XCTAssertTrue(next.waitForExistence(timeout: 5))
        XCTAssertTrue(element("task-1", in: next).exists, "the row is there, by its identifier")
        XCTAssertTrue(element("heading-next", in: next).exists, "with the bucket's heading")
        XCTAssertTrue(element("task-2", in: element("list-side", in: queue)).exists)

        let board = openWindow("open-board", title: "Board")
        XCTAssertTrue(element("list-next", in: board).waitForExistence(timeout: 5))
        XCTAssertTrue(element("task-1", in: element("list-next", in: board)).exists)
        XCTAssertTrue(element("task-2", in: element("list-side", in: board)).exists)
    }
}
