import Vision
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

    /// With only Settings open, a change that cannot be saved says so there.
    func testASettingThatCannotBeSavedSaysSo() throws {
        try launch()
        // A folder where the file goes: no write can replace it, and the app
        // cannot put it right as it can a folder's permissions.
        let blocked = dataHome.appendingPathComponent("queue-focus/settings.json/in-the-way")
        try FileManager.default.createDirectory(at: blocked, withIntermediateDirectories: true)
        let settings = openSettings()
        settings.descendants(matching: .any)["setting-quiet-hours"].firstMatch.click()
        let message = settings.descendants(matching: .any)["message-line"].firstMatch
        XCTAssertTrue(message.waitForExistence(timeout: 10), "the failed save shows in Settings")
    }

    /// The pickers show the times stored, whatever the clocks do today and
    /// wherever the Mac is. Their accessibility value is a moment, not what
    /// is on screen, so the screen is read.
    func testQuietHoursShowTheTimesStored() throws {
        try launch(settings: ["quiet_hours": true, "quiet_from": "02:30", "quiet_to": "18:45"])
        let settings = openSettings()
        let from = settings.datePickers["setting-quiet-from"]
        let to = settings.datePickers["setting-quiet-to"]
        XCTAssertTrue(from.waitForExistence(timeout: 5))
        let shown = try "\(readText(from)) / \(readText(to))"
        XCTAssertTrue(shown.contains("2:30") && (shown.contains("18:45") || shown.contains("6:45")), shown)
    }

    /// The text in `element` as a person sees it.
    private func readText(_ element: XCUIElement) throws -> String {
        let image = element.screenshot().image
        let cgImage = try XCTUnwrap(image.cgImage(forProposedRect: nil, context: nil, hints: nil))
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = false
        try VNImageRequestHandler(cgImage: cgImage).perform([request])
        return (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
    }
}
