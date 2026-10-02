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

    /// What one audit pass found: the issues that fail, the text it set
    /// aside because its scroll view had part of it out of view, and the
    /// text it saw whole inside its scroll view.
    private struct AuditPass {
        var failures: [String] = []
        var setAside: Set<String> = []
        var seenWhole: Set<String> = []
    }

    private func text(of element: any XCUIElementAttributes) -> String {
        (element.value as? String).flatMap { $0.isEmpty ? nil : $0 } ?? element.label
    }

    /// An element of one snapshot of the app, and everything inside it.
    private func everything(in node: any XCUIElementSnapshot) -> [any XCUIElementSnapshot] {
        [node] + node.children.flatMap { everything(in: $0) }
    }

    /// What the app names a note of Settings: one Text in one colour.
    private static let settingNote = "setting-note"

    /// One piece of text from pass to pass: its identifier where it has one
    /// of its own (which also holds while a countdown's words change),
    /// otherwise its words where they start across the line, which scrolling
    /// up and down does not move.
    private func key(identifier: String, text: String, frame: CGRect) -> String {
        identifier.isEmpty || identifier == Self.settingNote ? "\(text) @\(Int(frame.minX.rounded()))" : identifier
    }

    /// XCTest's audit of what is on screen, for the two kinds of issue worth
    /// failing on: text too faint to read, and controls VoiceOver cannot
    /// tell apart. What is let through is named. The system's: the root
    /// group of a window's hosting view, the Touch Bar and its items, a text
    /// field's completions window, the thumb of a named slider, a window's
    /// own title, and text with no part inside any window. And a note of
    /// Settings whose own pixels measure 4.5 to 1 or more: the audit
    /// misjudges some wrapped notes, so its verdict on them is checked
    /// (`PixelContrast`), and the measure logged. Text its
    /// scroll view shows only part of is set aside, to be measured whole in
    /// another pass (`expectClean` holds the test to that). Every issue goes
    /// to the log, so a new one is seen; an audit that fails attaches
    /// pictures of the screen, which the tests never keep, so the issues are
    /// gathered here instead.
    private func auditPass(_ surface: String) throws -> AuditPass {
        var pass = AuditPass()
        // One snapshot, so text that changes each second (the clock) cannot
        // go between reading the list and reading an element of it.
        let nodes = everything(in: try app.snapshot())
        let touchBarFrame = nodes.first { $0.elementType == .touchBar }.map { $0.frame.insetBy(dx: -4, dy: -4) } ?? .null
        let windows = nodes.filter { $0.elementType == .window }.map { (frame: $0.frame, title: $0.title) }
        let surfaces = windows.map(\.frame) + nodes.filter { $0.elementType == .popover }.map(\.frame)
        let namedSliders = nodes.filter { $0.elementType == .slider && !$0.label.isEmpty }.map(\.frame)
        let scrolling = nodes.filter { $0.elementType == .scrollView }.map { view in
            (frame: view.frame, texts: everything(in: view).filter { $0.elementType == .staticText }.map { element in
                (frame: element.frame, text: text(of: element), key: key(identifier: element.identifier, text: text(of: element),
                                                                         frame: element.frame))
            })
        }
        for view in scrolling {
            for text in view.texts where view.frame.contains(text.frame) {
                pass.seenWhole.insert(text.key)
            }
        }
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
                              surfaces.contains(where: { same($0, frame) }) {
                        reason = "a window's hosting root"
                    } else if element.elementType == .window, element.identifier.hasPrefix("SafariPlatformSupport") {
                        reason = "a text field's completions"
                    } else if element.elementType == .valueIndicator,
                              namedSliders.contains(where: { $0.insetBy(dx: -2, dy: -2).contains(frame) }) {
                        reason = "a named slider's thumb"
                    }
                case .contrast:
                    let value = self.text(of: element)
                    // The scroll view this text is content of, holding the
                    // whole of its width but not of its height.
                    let cutOff = scrolling.contains { view in
                        view.texts.contains { same($0.frame, frame) && $0.text == value }
                            && frame.minX >= view.frame.minX - 1 && frame.maxX <= view.frame.maxX + 1
                            && !(frame.minY >= view.frame.minY - 1 && frame.maxY <= view.frame.maxY + 1)
                    }
                    if !surfaces.contains(where: { $0.intersects(frame) }) {
                        reason = "outside every window"
                    } else if cutOff {
                        reason = "partly scrolled out of view, measured whole in another pass"
                        pass.setAside.insert(self.key(identifier: element.identifier, text: value, frame: frame))
                    } else if element.elementType == .staticText, windows.contains(where: { window in
                        !window.title.isEmpty && value == window.title
                            && window.frame.contains(frame) && frame.maxY <= window.frame.minY + 34
                    }) {
                        reason = "the window's title"
                    } else if element.elementType == .staticText, element.identifier == Self.settingNote,
                              element.exists,
                              let picture = element.screenshot().image.cgImage(forProposedRect: nil, context: nil, hints: nil),
                              let measured = PixelContrast.measure(picture), measured >= 4.5 {
                        reason = String(format: "its pixels measure %.2f to 1", measured)
                    }
                default:
                    break
                }
            }
            let what = issue.element.map { $0.debugDescription.split(separator: "\n").prefix(3).joined(separator: " / ") } ?? "no element"
            print("AUDIT \(surface): \(reason.map { "ignored (\($0))" } ?? "FAILED"): \(issue.compactDescription) — \(what)")
            if reason == nil {
                pass.failures.append("\(issue.compactDescription): \(what)")
            }
            return true
        }
        return pass
    }

    /// No issue failed, and every line set aside was seen whole in some pass,
    /// where the audit measured it.
    private func expectClean(_ passes: [AuditPass], file: StaticString = #filePath, line: UInt = #line) {
        if !TestEnvironment.isAuditBaseline {
            XCTExpectFailure("the audit's findings are macOS 27's", options: .nonStrict())
        }
        let failures = passes.flatMap(\.failures)
        XCTAssertTrue(failures.isEmpty, failures.joined(separator: "\n"), file: file, line: line)
        let setAside = passes.reduce(into: Set<String>()) { $0.formUnion($1.setAside) }
        let seenWhole = passes.reduce(into: Set<String>()) { $0.formUnion($1.seenWhole) }
        let unmeasured = setAside.subtracting(seenWhole)
        XCTAssertTrue(unmeasured.isEmpty, "never seen whole by the audit: \(unmeasured.sorted())", file: file, line: line)
    }

    /// A surface audited once: nothing can be measured in a later pass, so
    /// nothing may be set aside.
    private func audit(_ surface: String, file: StaticString = #filePath, line: UInt = #line) throws {
        let pass = try auditPass(surface)
        expectClean([pass], file: file, line: line)
        XCTAssertTrue(pass.setAside.isEmpty, "cut off in a surface audited once: \(pass.setAside.sorted())", file: file, line: line)
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
        var passes: [AuditPass] = []
        for page in 1...4 {
            passes.append(try auditPass("\(name), part \(page)"))
            form.scroll(byDeltaX: 0, deltaY: -400)
        }
        expectClean(passes)
    }

    func testSettingsPassesTheAudit() throws {
        // Light whatever the Mac is set to; the dark run is below.
        try launch(with: seed, settings: ["theme": "light"])
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
