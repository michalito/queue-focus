import XCTest

/// What VoiceOver finds: every row and control there, under its name.
@MainActor
final class AccessibilityUITests: AppUITestCase {
    private func element(_ id: String, in parent: XCUIElement) -> XCUIElement {
        parent.descendants(matching: .any)[id].firstMatch
    }

    private let seed: [(String, String)] = [
        ("ship v0.1 #w", "now"), ("write notes #p", "next"), ("a side", "side"),
        ("a later", "later"), ("a later for work #w", "later"), ("a later for home #p", "later"),
    ]

    /// XCTest's audit of what is on screen, failing on the two kinds of issue
    /// worth failing on: text too faint to read, and controls VoiceOver
    /// cannot tell apart. What is let through is named, and is the system's:
    /// the root group of a window's hosting view, the Touch Bar and its
    /// items, a text field's completions window, the thumb of a named slider,
    /// a window's title bar (its title, and content scrolled under it), text
    /// a scroll view cuts off (measured whole once scrolled into view, which
    /// the Settings audit does), and text with no part inside any window. Every
    /// issue goes to the log, so a new one is seen, and the test fails on its
    /// own account: an audit that fails attaches pictures of the screen,
    /// which the tests never keep.
    private func audit(_ surface: String, file: StaticString = #filePath, line: UInt = #line) throws {
        var failures: [String] = []
        let touchBar = app.touchBars.firstMatch
        let touchBarFrame = touchBar.exists ? touchBar.frame.insetBy(dx: -4, dy: -4) : .null
        let surfaces = (app.windows.allElementsBoundByIndex + app.popovers.allElementsBoundByIndex)
            .map { (frame: $0.frame, title: $0.title) }
        let namedSliders = app.sliders.allElementsBoundByIndex.filter { !$0.label.isEmpty }.map(\.frame)
        let scrollViews = app.scrollViews.allElementsBoundByIndex.map(\.frame)
        let same = { (a: CGRect, b: CGRect) in abs(a.minX - b.minX) < 2 && abs(a.minY - b.minY) < 2
            && abs(a.width - b.width) < 2 && abs(a.height - b.height) < 2 }
        try app.performAccessibilityAudit(for: [.contrast, .sufficientElementDescription]) { issue in
            var reason: String?
            if let element = issue.element {
                let frame = element.frame
                switch issue.auditType {
                case .sufficientElementDescription:
                    if element.elementType == .touchBar || touchBarFrame.contains(frame) {
                        reason = "the Touch Bar"
                    } else if element.elementType == .group, element.identifier.isEmpty,
                              surfaces.contains(where: { same($0.frame, frame) }) {
                        reason = "a window's hosting root"
                    } else if element.elementType == .window, element.identifier.hasPrefix("SafariPlatformSupport") {
                        reason = "a text field's completions"
                    } else if element.elementType == .valueIndicator,
                              namedSliders.contains(where: { $0.insetBy(dx: -2, dy: -2).contains(frame) }) {
                        reason = "a named slider's thumb"
                    }
                case .contrast:
                    if !surfaces.contains(where: { $0.frame.intersects(frame) }) {
                        reason = "outside every window"
                    } else if scrollViews.contains(where: { $0.intersects(frame) && !$0.contains(frame) }) {
                        // Measured on a sliver; it is measured whole once
                        // scrolled into view.
                        reason = "cut off by its scroll view"
                    } else if surfaces.contains(where: { surface in
                        surface.frame.contains(frame) && frame.maxY <= surface.frame.minY + 34
                    }) {
                        // The band at the top of a window: its title, or
                        // content scrolled up under it, which the system fades.
                        reason = "a window's title bar"
                    }
                default:
                    break
                }
            }
            let what = issue.element.map { $0.debugDescription.split(separator: "\n").prefix(3).joined(separator: " / ") } ?? "no element"
            print("AUDIT \(surface): \(reason.map { "ignored (\($0))" } ?? "FAILED"): \(issue.compactDescription) — \(what)")
            if reason == nil {
                failures.append("\(issue.compactDescription): \(what)")
            }
            return true
        }
        if !TestEnvironment.isAuditBaseline {
            XCTExpectFailure("the audit's findings are macOS 27's", options: .nonStrict())
        }
        XCTAssertTrue(failures.isEmpty, failures.joined(separator: "\n"), file: file, line: line)
    }

    private func openSettings() -> XCUIElement {
        _ = openPopover()
        click(app.menuButtons["gear-menu"])
        click(app.menuItems["gear-settings"])
        let settings = app.windows["com_apple_SwiftUI_Settings_window"]
        XCTAssertTrue(settings.waitForExistence(timeout: 5))
        return settings
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

    /// Settings scrolls: each part is audited once in view.
    private func auditSettings(_ name: String) throws {
        let settings = openSettings()
        let form = settings.scrollViews.firstMatch
        for page in 1...4 {
            try audit("\(name), part \(page)")
            form.scroll(byDeltaX: 0, deltaY: -400)
        }
    }

    func testSettingsPassesTheAudit() throws {
        try launch(with: seed)
        try auditSettings("settings")
    }

    func testInTheDarkSettingsAndTheBoardPassTheAudit() throws {
        try launch(with: seed, settings: ["theme": "dark"])
        try auditSettings("settings, dark")
        app.typeKey("w", modifierFlags: .command)
        _ = openWindow("open-board", title: "Board")
        try audit("board, dark")
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
        XCTAssertEqual(element("later-shelf", in: queue).label, "Later, 3")
        XCTAssertEqual(queue.buttons["Settings"].firstMatch.label, "Settings")

        // The popover: its menu, a Side card's buttons, and the message line.
        let field = openPopover()
        let popover = app.popovers.firstMatch
        XCTAssertEqual(app.menuButtons["gear-menu"].title, "Settings and more")
        // A Side card's buttons show under the pointer.
        element("side-3", in: popover).hover()
        XCTAssertTrue(element("done-3", in: popover).waitForExistence(timeout: 5))
        XCTAssertEqual(element("promote-3", in: popover).label, "Make current")
        XCTAssertEqual(element("done-3", in: popover).label, "Done")
        // Markers and no title: refused, and said so.
        field.typeText("#w @later\r")
        let message = element("message-line", in: popover)
        XCTAssertTrue(message.waitForExistence(timeout: 5))
        XCTAssertTrue(message.buttons["Dismiss"].exists, "the message line holds its Dismiss button")
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
