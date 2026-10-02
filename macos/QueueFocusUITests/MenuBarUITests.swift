import XCTest

/// The real status item and popover, on a data directory of the test's own.
@MainActor
final class MenuBarUITests: XCTestCase {
    private var app: XCUIApplication!
    private var dataHome: URL!

    /// Launch the app on a fresh data directory, removed after the test,
    /// holding `tasks` if given: `(title, bucket)` pairs.
    private func launch(with tasks: [(String, String)] = [], restoringState: Bool = false) throws {
        continueAfterFailure = false
        let dataHome = FileManager.default.temporaryDirectory
            .appendingPathComponent("qf-ui-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dataHome, withIntermediateDirectories: true)
        if !tasks.isEmpty {
            let dir = dataHome.appendingPathComponent("queue-focus")
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let file: [String: Any] = [
                "next_id": tasks.count + 1,
                "tasks": tasks.enumerated().map { index, task in
                    ["id": index + 1, "title": task.0, "bucket": task.1, "created_at": 0]
                },
            ]
            try JSONSerialization.data(withJSONObject: file).write(to: dir.appendingPathComponent("tasks.json"))
        }
        let app = XCUIApplication()
        app.launchEnvironment["XDG_DATA_HOME"] = dataHome.path
        app.launchArguments += ["-allowSecondInstance", "YES"]
        if restoringState {
            // As for someone who keeps windows when quitting an app.
            app.launchArguments += ["-ApplePersistenceIgnoreState", "NO", "-NSQuitAlwaysKeepsWindows", "YES"]
        }
        app.launch()
        addTeardownBlock { @MainActor in
            app.terminate()
            try? FileManager.default.removeItem(at: dataHome)
        }
        self.app = app
        self.dataHome = dataHome
        XCTAssertTrue(statusItem.waitForExistence(timeout: 10), "the status item appears")
        XCTAssertEqual(app.windows.count, 0, "a menu bar app opens no window at launch: \(app.windows.debugDescription)")
    }

    /// The saved queue, as the engine wrote it.
    private func storedTitles() throws -> [String] {
        let data = try Data(contentsOf: dataHome.appendingPathComponent("queue-focus/tasks.json"))
        let file = try JSONSerialization.jsonObject(with: data) as? [String: Any]
        let tasks = file?["tasks"] as? [[String: Any]] ?? []
        return tasks.compactMap { $0["title"] as? String }
    }

    private var statusItem: XCUIElement {
        app.statusItems["queue-focus-status-item"]
    }

    private var statusTitle: String {
        statusItem.title.isEmpty ? (statusItem.value as? String ?? "") : statusItem.title
    }

    private func openPopover() -> XCUIElement {
        statusItem.click()
        let field = app.textFields["add-field"]
        XCTAssertTrue(field.waitForExistence(timeout: 5), "the popover opens with its add field")
        return field
    }

    private func waitForTitle(containing text: String, file: StaticString = #filePath, line: UInt = #line) {
        let predicate = NSPredicate { _, _ in MainActor.assumeIsolated { self.statusTitle.contains(text) } }
        let found = XCTNSPredicateExpectation(predicate: predicate, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [found], timeout: 5), .completed,
                       "status title \(statusTitle.debugDescription) never contained \(text)", file: file, line: line)
    }

    func testAddingCompletingAndUndoingFromThePopover() throws {
        try launch()
        waitForTitle(containing: "no task")

        // Return adds to Next; the `!` marker makes it the current task.
        let field = openPopover()
        field.typeText("first task !\r")
        waitForTitle(containing: "first task")

        _ = openPopover()
        app.buttons["done-current"].click()
        XCTAssertTrue(app.buttons["undo"].waitForExistence(timeout: 5), "Done offers Undo")
        waitForTitle(containing: "no task")

        app.buttons["undo"].click()
        waitForTitle(containing: "first task")
        XCTAssertEqual(try storedTitles(), ["first task"], "the engine saved the queue in the test's directory")
    }

    func testCommandReturnAddsTheTaskAsCurrent() throws {
        try launch()
        let field = openPopover()
        field.typeText("queued\r")
        waitForTitle(containing: "no task")
        let again = openPopover()
        again.typeText("right now")
        again.typeKey(.return, modifierFlags: .command)
        waitForTitle(containing: "right now")
        XCTAssertEqual(try storedTitles().sorted(), ["queued", "right now"], "one press, one task")
    }

    func testARightClickPausesTheTimer() throws {
        try launch()
        openPopover().typeText("timed !\r")
        waitForTitle(containing: "timed")
        statusItem.rightClick()
        waitForTitle(containing: "❚❚")
        statusItem.rightClick()
        let resumed = NSPredicate { _, _ in MainActor.assumeIsolated { !self.statusTitle.contains("❚❚") } }
        XCTAssertEqual(XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: resumed, object: nil)], timeout: 5), .completed)
    }

    func testEscapeClosesThePopoverAndAddsNothing() throws {
        try launch()
        let field = openPopover()
        field.typeText("not wanted")
        field.typeKey(.escape, modifierFlags: [])
        let gone = NSPredicate { _, _ in MainActor.assumeIsolated { !field.exists } }
        XCTAssertEqual(XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: gone, object: nil)], timeout: 5), .completed,
                       "Escape closes the popover")
        XCTAssertFalse(FileManager.default.fileExists(atPath: dataHome.appendingPathComponent("queue-focus/tasks.json").path),
                       "nothing was added, so nothing was saved")
    }

    func testASideCardOffersItsActionsAndCompletesWithUndo() throws {
        try launch()
        openPopover().typeText("beside it @side\r")
        let card = app.descendants(matching: .any)["side-1"]
        _ = openPopover()
        XCTAssertTrue(card.waitForExistence(timeout: 5), "the Side card is in the popover")
        card.hover()
        app.buttons["done-1"].click()
        XCTAssertTrue(app.buttons["undo"].waitForExistence(timeout: 5), "a Side task done offers Undo")
        app.buttons["undo"].click()
        XCTAssertTrue(card.waitForExistence(timeout: 5), "Undo puts it back")
    }

    /// A long Side list scrolls inside the popover instead of growing it past the screen.
    func testALongSideListScrollsInsteadOfGrowing() throws {
        try launch(with: (1...25).map { ("side task \($0)", "side") })
        _ = openPopover()
        XCTAssertTrue(app.descendants(matching: .any)["side-1"].waitForExistence(timeout: 5))
        let popover = app.popovers.firstMatch
        XCTAssertTrue(popover.exists)
        XCTAssertLessThan(popover.frame.height, 560, "popover \(popover.frame) grew with its list")
    }

    func testThePopoverOpensTheWindows() throws {
        try launch()
        _ = openPopover()
        app.buttons["open-queue"].click()
        XCTAssertTrue(app.windows["Queue"].waitForExistence(timeout: 5), "Queue opens from the popover")
        _ = openPopover()
        app.buttons["open-board"].click()
        XCTAssertTrue(app.windows["Board"].waitForExistence(timeout: 5), "Board opens from the popover")
        _ = openPopover()
        app.menuButtons["gear-menu"].click()
        app.menuItems["gear-settings"].click()
        XCTAssertTrue(app.windows["com_apple_SwiftUI_Settings_window"].waitForExistence(timeout: 5),
                      "Settings opens from the gear menu")
    }

    /// Quitting with windows open and opening the app again opens none: a
    /// menu bar app opens nothing until asked.
    func testNoWindowComesBackAfterQuitting() throws {
        // UI tests launch with state restoration off; this one needs it on.
        try launch(restoringState: true)
        _ = openPopover()
        app.buttons["open-queue"].click()
        XCTAssertTrue(app.windows["Queue"].waitForExistence(timeout: 5))
        _ = openPopover()
        app.menuButtons["gear-menu"].click()
        app.menuItems["gear-quit"].click()
        XCTAssertTrue(app.wait(for: .notRunning, timeout: 10), "Quit quits")

        app.launch()
        XCTAssertTrue(statusItem.waitForExistence(timeout: 10))
        XCTAssertEqual(app.windows.count, 0, "no window came back: \(app.windows.debugDescription)")
    }

    func testASettingsChangeReachesTheFile() throws {
        try launch()
        _ = openPopover()
        app.menuButtons["gear-menu"].click()
        app.menuItems["gear-settings"].click()
        let settings = app.windows["com_apple_SwiftUI_Settings_window"]
        XCTAssertTrue(settings.waitForExistence(timeout: 5))
        settings.descendants(matching: .any)["setting-show-timer"].firstMatch.click()
        let file = dataHome.appendingPathComponent("queue-focus/settings.json")
        let written = NSPredicate { _, _ in
            guard let data = try? Data(contentsOf: file),
                  let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
            else { return false }
            return json["show_timer"] as? Bool == false
        }
        XCTAssertEqual(XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: written, object: nil)], timeout: 5), .completed,
                       "the change reached settings.json within the writer's second")
    }
}
