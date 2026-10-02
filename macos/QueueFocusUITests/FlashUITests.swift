import XCTest

/// The flash, drawn for real: over everything, then gone.
@MainActor
final class FlashUITests: AppUITestCase {
    private var overlay: XCUIElement {
        app.descendants(matching: .any)["flash-overlay"].firstMatch
    }

    private func waitForTheFlashToGo(file: StaticString = #filePath, line: UInt = #line) {
        let overlay = overlay
        // 1.8 s for the longest run, and room for a busy machine.
        XCTAssertTrue(overlay.waitForNonExistence(timeout: 5), "the flash goes once it has played", file: file, line: line)
    }

    func testThePreviewDrawsAStyleThenTakesItDown() throws {
        try launch(arguments: ["-flashPreview", "edges"])
        XCTAssertTrue(overlay.waitForExistence(timeout: 6), "two seconds in, the flash is drawn")
        XCTAssertEqual(overlay.label, "Queue Focus flash: NOW, Write the quarterly report, 23m")
        waitForTheFlashToGo()
    }

    func testFlashNowDrawsTheCurrentTask() throws {
        try launch(with: [("ship v0.1", "now")])
        _ = openPopover()
        click(app.menuButtons["gear-menu"])
        click(app.menuItems["gear-settings"])
        let settings = app.windows["com_apple_SwiftUI_Settings_window"]
        XCTAssertTrue(settings.waitForExistence(timeout: 5))
        click(settings.buttons["flash-now"])
        XCTAssertTrue(overlay.waitForExistence(timeout: 3), "Flash now draws one at once")
        XCTAssertTrue(overlay.label.hasPrefix("Queue Focus flash: NOW, ship v0.1"), overlay.label)
        // The keyboard stays where it was: ⌘W, while the flash is up, closes
        // Settings.
        app.typeKey("w", modifierFlags: .command)
        XCTAssertTrue(settings.waitForNonExistence(timeout: 2), "Settings kept the keyboard")
        waitForTheFlashToGo()
    }
}
