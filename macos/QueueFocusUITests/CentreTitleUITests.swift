import XCTest

/// The task in the centre of the menu bar: a label there carries the title,
/// and the status item keeps the dot. On a data directory of the test's own,
/// with the setting in the launch arguments, so the user's stays as it is.
@MainActor
final class CentreTitleUITests: AppUITestCase {
    private var label: XCUIElement {
        app.buttons["queue-focus-centre-title"]
    }

    private var screen: CGRect {
        NSScreen.screens[0].frame
    }

    private func launchCentred(with tasks: [(String, String)]) throws {
        try XCTSkipIf(NSScreen.screens[0].safeAreaInsets.top > 0, "a notch takes the centre of this screen's menu bar")
        try launch(with: tasks, arguments: ["-menuBarTitleCentred", "YES"])
        XCTAssertTrue(label.waitForExistence(timeout: 5), "the label is in the menu bar")
    }

    private func wait(_ description: String, timeout: TimeInterval = 5, file: StaticString = #filePath, line: UInt = #line,
                      until condition: @escaping @MainActor () -> Bool) {
        let predicate = NSPredicate { _, _ in MainActor.assumeIsolated { condition() } }
        XCTAssertEqual(XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: predicate, object: nil)], timeout: timeout),
                       .completed, description, file: file, line: line)
    }

    func testTheTitleIsInTheMiddleAndTheItemKeepsTheDot() throws {
        try launchCentred(with: [("ship the release", "now")])
        XCTAssertTrue(label.label.contains("ship the release"), label.label)
        wait("the item is down to its dot: \(statusTitle.debugDescription)") { self.statusTitle == "●" }
        waitForTheMenuBarToSettle()
        XCTAssertEqual(label.frame.midX, screen.midX, accuracy: 1, "in the middle of the screen")
        XCTAssertEqual(label.frame.minY, 0, accuracy: 1, "in the menu bar")
        XCTAssertLessThan(label.frame.maxX, statusItem.frame.minX, "clear of the item: \(label.frame), \(statusItem.frame)")
        XCTAssertEqual(openWindows.count, 0, "and no window of the app's is open")
    }

    func testWithoutTheSettingTheItemHasTheTitle() throws {
        try launch(with: [("ship the release", "now")])
        waitForTitle(containing: "ship the release")
        XCTAssertFalse(label.exists, "no label unless Settings asks for one")
    }

    /// The popover is transient, and the label is outside it: the second
    /// click closes it, and must not open it again.
    func testAClickOnTheLabelOpensThePopoverUnderItAndAnotherClosesIt() throws {
        try launchCentred(with: [("ship the release", "now")])
        label.click()
        let field = app.textFields["add-field"]
        XCTAssertTrue(field.waitForExistence(timeout: 5), "a click on the label opens the popover")
        XCTAssertEqual(app.popovers.firstMatch.frame.midX, screen.midX, accuracy: 2, "under the label")

        label.click()
        XCTAssertTrue(field.waitForNonExistence(timeout: 5), "the second click closes the popover")
        let reopened = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in MainActor.assumeIsolated { field.exists } },
                                                 object: nil)
        reopened.isInverted = true
        wait(for: [reopened], timeout: 2)

        label.click()
        XCTAssertTrue(field.waitForExistence(timeout: 5), "and a third opens it again")
    }

    /// The label's middle is the screen's whatever its words, so the popover
    /// under it stays where it is when a task is done.
    func testThePopoverStaysUnderTheLabelWhenTheTitleChanges() throws {
        try launchCentred(with: [("a task with quite a long title to it", "now"), ("next", "next")])
        label.click()
        let popover = app.popovers.firstMatch
        XCTAssertTrue(popover.waitForExistence(timeout: 5))
        let opened = popover.frame
        let wide = label.frame

        click(app.buttons["done-current"])
        wait("the label has the next task: \(label.label)") { self.label.label.contains("next") }
        XCTAssertLessThan(label.frame.width, wide.width - 40, "the label is narrower")
        XCTAssertEqual(label.frame.midX, screen.midX, accuracy: 1, "and still in the middle")
        XCTAssertTrue(app.buttons["undo"].waitForExistence(timeout: 5))
        XCTAssertEqual(popover.frame.midX, opened.midX, accuracy: 1, "the popover did not move")
        XCTAssertEqual(popover.frame.minY, opened.minY, accuracy: 1)
    }

    func testARightClickOnTheLabelPausesTheTimer() throws {
        try launchCentred(with: [("timed", "now")])
        label.rightClick()
        wait("paused: \(label.label)") { self.label.label.contains("paused") }
        label.rightClick()
        wait("running again: \(label.label)") { !self.label.label.contains("paused") }
    }

    /// A click on the dot still opens the popover, there.
    func testTheDotStillOpensThePopover() throws {
        try launchCentred(with: [("ship the release", "now")])
        wait("the item is down to its dot") { self.statusTitle == "●" }
        waitForTheMenuBarToSettle()
        _ = openPopover()
        let popover = app.popovers.firstMatch
        let free = popover.frame.minX > screen.minX + 40 && popover.frame.maxX < screen.maxX - 40
        if free {
            XCTAssertEqual(popover.frame.midX, statusItem.frame.midX, accuracy: 2, "from the dot, not the label")
        }
    }

    /// In a full-screen space the menu bar is out of view, and the label
    /// with it: the item takes the title back, for when the bar is shown.
    func testInAFullScreenSpaceTheItemTakesTheTitleBack() throws {
        try launchCentred(with: [("ship the release", "now")])
        wait("the item is down to its dot") { self.statusTitle == "●" }
        let queue = openWindow("open-queue", title: "Queue")
        queue.buttons[XCUIIdentifierFullScreenWindow].click()
        // Going to a space of its own and back each take a moment.
        wait("the item has the title in a full-screen space: \(statusTitle.debugDescription)", timeout: 15) {
            self.statusTitle.contains("ship the release")
        }
        // Closing the window ends its space.
        app.typeKey("w", modifierFlags: .command)
        wait("and gives it up again on the desktop: \(statusTitle.debugDescription)", timeout: 15) { self.statusTitle == "●" }
    }
}
