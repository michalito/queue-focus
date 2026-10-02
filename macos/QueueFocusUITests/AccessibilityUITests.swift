import XCTest

/// What VoiceOver finds: every row and control there, under its name.
@MainActor
final class AccessibilityUITests: AppUITestCase {
    private func element(_ id: String, in parent: XCUIElement) -> XCUIElement {
        parent.descendants(matching: .any)[id].firstMatch
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
