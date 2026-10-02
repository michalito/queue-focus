import XCTest

/// The global shortcuts and the `queuefocus:` links, on the real app. The
/// shortcuts are the test ones `AppUITestCase` moves them to, and
/// notifications are off, so a completion is offered in the popover.
@MainActor
final class HotkeyUITests: AppUITestCase {
    private let seed: [(String, String)] = [("now task", "now"), ("first next", "next")]

    private func waitForNoWindow(_ title: String, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertTrue(app.windows[title].waitForNonExistence(timeout: 5), "\(title) closed", file: file, line: line)
    }

    func testTheQueueShortcutShowsTheQueueThenHidesIt() throws {
        try launch(with: seed)
        pressShortcut("toggleQueue")
        XCTAssertTrue(app.windows["Queue"].waitForExistence(timeout: 5), "the shortcut shows the Queue")
        pressShortcut("toggleQueue")
        waitForNoWindow("Queue")
        pressShortcut("toggleQueue")
        XCTAssertTrue(app.windows["Queue"].waitForExistence(timeout: 5), "and shows it again")
    }

    func testTheBoardShortcutShowsTheBoard() throws {
        try launch(with: seed)
        pressShortcut("showBoard")
        XCTAssertTrue(app.windows["Board"].waitForExistence(timeout: 5))
    }

    func testTheQuickAddShortcutOpensQuickAdd() throws {
        try launch(with: seed)
        pressShortcut("quickAdd")
        XCTAssertTrue(app.textFields["quick-add-field"].waitForExistence(timeout: 5))
    }

    /// With notifications off, the completion is offered where it can be
    /// seen: the popover opens on its Undo row.
    func testTheCompleteShortcutCompletesAndThePopoverOffersUndo() throws {
        try launch(with: seed)
        pressShortcut("completeCurrent")
        waitForStore("the current task is done and Next's first took over") {
            try self.stored("now") == ["first next"]
        }
        click(app.buttons["undo"])
        waitForStore("Undo puts it back") {
            try self.stored("now") == ["now task"] && self.stored("next") == ["first next"]
        }
    }

    func testTheCompleteShortcutOnAnEmptyNowOpensThePopover() throws {
        try launch(with: [("only next", "next")])
        pressShortcut("completeCurrent")
        XCTAssertTrue(app.textFields["add-field"].waitForExistence(timeout: 5), "the popover shows Now is empty")
        XCTAssertEqual(try stored("next"), ["only next"], "and nothing was completed")
    }

    /// A link that starts the app waits for the queue to open. (XCTest opens
    /// every link in a fresh copy of the app, with the test's arguments, so
    /// the app is closed before each: one engine per queue. A link to the
    /// running app is the unit tests'.)
    func testALinkThatStartsTheAppIsFollowedOnceItIsOpen() throws {
        try launch(with: seed)
        app.terminate()
        app.open(try XCTUnwrap(URL(string: "queuefocus://add?text=from%20a%20link&now=1")))
        XCTAssertTrue(statusItem.waitForExistence(timeout: 10), "the link started the app")
        waitForStore("the link added the task as current") {
            try self.stored("now") == ["from a link"]
        }
        app.terminate()
        app.open(try XCTUnwrap(URL(string: "queuefocus://show?view=board")))
        XCTAssertTrue(app.windows["Board"].waitForExistence(timeout: 10), "a link opens a view")
    }

    /// Settings shows each action with its keys: here the test ones, in the
    /// order the actions are listed.
    func testSettingsListsTheShortcuts() throws {
        try launch()
        _ = openPopover()
        click(app.menuButtons["gear-menu"])
        click(app.menuItems["gear-settings"])
        let settings = app.windows["com_apple_SwiftUI_Settings_window"]
        XCTAssertTrue(settings.waitForExistence(timeout: 5))
        for title in ["Show or hide the Queue", "Quick add", "Show the Board", "Complete the current task"] {
            XCTAssertTrue(settings.staticTexts[title].waitForExistence(timeout: 5), title)
        }
        let recorders = settings.searchFields.matching(NSPredicate(format: "placeholderValue == %@", "Record Shortcut"))
        let shown = recorders.allElementsBoundByIndex.map { $0.value as? String ?? "" }
        let expected = Self.testShortcuts.map { "⌃⌥⇧" + $0.key.rawValue.uppercased() }
        XCTAssertEqual(shown, expected)
        XCTAssertTrue(settings.buttons["Restore Defaults"].exists)
    }
}
