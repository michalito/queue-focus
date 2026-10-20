import XCTest

/// The floating quick add field.
///
/// XCTest refuses to type into this panel: its accessibility does not report
/// the field as focused, though AppKit has made the field the first
/// responder of the key panel. What is typed there is covered by the model's
/// tests of `add`; these tests cover opening and closing.
@MainActor
final class QuickAddUITests: AppUITestCase {
    func testCommandNOpensItAndAnotherWindowClosesIt() throws {
        try launch()
        _ = openWindow("open-queue", title: "Queue")
        app.typeKey("n", modifierFlags: .command)
        let field = app.textFields["quick-add-field"]
        XCTAssertTrue(field.waitForExistence(timeout: 5), "Command-N opens the quick add field")

        app.windows["Queue"].click()
        let gone = NSPredicate { _, _ in MainActor.assumeIsolated { !field.exists } }
        XCTAssertEqual(XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: gone, object: nil)], timeout: 5),
                       .completed, "it goes when another window takes the keyboard")
    }
}
