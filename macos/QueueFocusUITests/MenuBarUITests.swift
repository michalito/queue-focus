import XCTest

/// The real status item and popover, on a data directory of the test's own.
@MainActor
final class MenuBarUITests: AppUITestCase {
    func testAddingCompletingAndUndoingFromThePopover() throws {
        try launch()
        waitForTitle(containing: "no task")

        // Return adds to Next; the `!` marker makes it the current task.
        let field = openPopover()
        field.typeText("first task !\r")
        waitForTitle(containing: "first task")

        _ = openPopover()
        click(app.buttons["done-current"])
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

    /// The popover closes on a click outside it, and the item is outside it:
    /// a second click on the item leaves it closed, not closed and opened
    /// again on the same click.
    func testClickingTheItemAgainClosesThePopover() throws {
        try launch()
        let field = openPopover()
        statusItem.click()
        XCTAssertTrue(field.waitForNonExistence(timeout: 5), "the second click closes the popover")
        let reopened = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in MainActor.assumeIsolated { field.exists } },
                                                 object: nil)
        reopened.isInverted = true
        wait(for: [reopened], timeout: 2)
        _ = openPopover()
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
        click(app.buttons["open-queue"])
        XCTAssertTrue(app.windows["Queue"].waitForExistence(timeout: 5), "Queue opens from the popover")
        _ = openPopover()
        click(app.buttons["open-board"])
        XCTAssertTrue(app.windows["Board"].waitForExistence(timeout: 5), "Board opens from the popover")
        _ = openPopover()
        click(app.menuButtons["gear-menu"])
        click(app.menuItems["gear-settings"])
        XCTAssertTrue(app.windows["com_apple_SwiftUI_Settings_window"].waitForExistence(timeout: 5),
                      "Settings opens from the gear menu")
    }

    /// Quitting with windows open and opening the app again opens none: a
    /// menu bar app opens nothing until asked.
    func testNoWindowComesBackAfterQuitting() throws {
        // UI tests launch with state restoration off; this one needs it on.
        try launch(restoringState: true)
        _ = openPopover()
        click(app.buttons["open-queue"])
        XCTAssertTrue(app.windows["Queue"].waitForExistence(timeout: 5))
        _ = openPopover()
        click(app.menuButtons["gear-menu"])
        click(app.menuItems["gear-quit"])
        XCTAssertTrue(app.wait(for: .notRunning, timeout: 10), "Quit quits")

        app.launch()
        XCTAssertTrue(statusItem.waitForExistence(timeout: 10))
        XCTAssertEqual(app.windows.count, 0, "no window came back: \(app.windows.debugDescription)")
    }

    func testASettingsChangeReachesTheFile() throws {
        try launch()
        _ = openPopover()
        click(app.menuButtons["gear-menu"])
        click(app.menuItems["gear-settings"])
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
