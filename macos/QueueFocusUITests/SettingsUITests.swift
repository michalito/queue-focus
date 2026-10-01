import XCTest

/// The Settings window, over a data directory of the test's own.
@MainActor
final class SettingsUITests: AppUITestCase {
    private func openSettings() -> XCUIElement {
        _ = openPopover()
        click(app.menuButtons["gear-menu"])
        click(app.menuItems["gear-settings"])
        let settings = app.windows["com_apple_SwiftUI_Settings_window"]
        XCTAssertTrue(settings.waitForExistence(timeout: 5), "Settings opens")
        return settings
    }

    private func settingsFile() throws -> [String: Any] {
        let data = try Data(contentsOf: dataHome.appendingPathComponent("queue-focus/settings.json"))
        return try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
    }

    func testTryItSaysWhyThereIsNoFlash() throws {
        try launch()
        let settings = openSettings()
        let status = settings.staticTexts["flash-status"]
        XCTAssertTrue(status.waitForExistence(timeout: 5))
        XCTAssertEqual(status.value as? String ?? status.label, "No flash: nothing in Now")
        XCTAssertFalse(settings.buttons["flash-now"].isEnabled, "nothing in Now to flash")
    }

    func testTryItCountsDownAndFlashNowRestartsTheWait() throws {
        try launch(with: [("focus", "now")])
        let settings = openSettings()
        let status = settings.staticTexts["flash-status"]
        let counting = NSPredicate { _, _ in
            MainActor.assumeIsolated { (status.value as? String ?? status.label).hasPrefix("Next flash in 1") }
        }
        XCTAssertEqual(XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: counting, object: nil)], timeout: 5), .completed,
                       "the countdown shows: \(status.label)")
        let button = settings.buttons["flash-now"]
        XCTAssertTrue(button.isEnabled)
        button.click()
        XCTAssertTrue((status.value as? String ?? status.label).hasPrefix("Next flash in 1"), "still counting after a flash")
    }

    func testQuietHoursReachTheFile() throws {
        try launch()
        let settings = openSettings()
        settings.descendants(matching: .any)["setting-quiet-hours"].firstMatch.click()
        waitForStore("quiet hours are on in settings.json") {
            try self.settingsFile()["quiet_hours"] as? Bool == true
        }
    }
}
