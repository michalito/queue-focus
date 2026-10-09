import AppKit
import Testing
@testable import QueueFocus

/// The popover is transient: a click on the status item, or on the label in
/// the centre, closes it before the click reaches either. That click must
/// not open it again; any other must.
@Suite struct PopoverClosingTests {
    typealias Closing = StatusItemController.Closing

    /// Where the click comes straight to the app, the popover closes on the
    /// click itself.
    @Test func theVeryClickThatClosedItDoesNotOpenIt() {
        let closing = Closing(at: 100.004, by: .leftMouseDown, eventTime: 100, over: .label)
        #expect(closing.wasMade(by: 100, on: .label))
        #expect(closing.wasMade(by: 100, on: .item), "whichever it reaches")
        #expect(!closing.wasMade(by: 100.2, on: .label), "the next click is another")
    }

    /// macOS 26 and later keep the menu bar to themselves: AppKit closes the
    /// popover on an event of its own, and the click on the item is handed
    /// on after, timed when it was handed on.
    @Test func aClickTheMenuBarHandsOnDoesNotOpenIt() {
        let closing = Closing(at: 776.431, by: .appKitDefined, eventTime: 776.380, over: .item)
        #expect(closing.wasMade(by: 776.457, on: .item), "26 thousandths after the close, as measured")
        #expect(closing.wasMade(by: 776.508, on: .item), "and 77")
        #expect(!closing.wasMade(by: 776.431 + Closing.handingOn + 0.01, on: .item), "a later click is another")
        #expect(!closing.wasMade(by: 776.457, on: .label), "and so is one somewhere else")
    }

    /// A click in another app closes the popover in the same way, the
    /// pointer nowhere near the item.
    @Test func aClickElsewhereLeavesTheNextOneToOpenIt() {
        let closing = Closing(at: 50, by: .appKitDefined, eventTime: 49.95, over: nil)
        #expect(!closing.wasMade(by: 50.1, on: .item))
        #expect(!closing.wasMade(by: 50.1, on: .label))
    }

    /// Escape, Return after a task's title, a button in the popover, the
    /// app's own closing of it: a click straight after is a click to open.
    @Test func afterAnyOtherCloseTheNextClickOpensIt() {
        for kind: NSEvent.EventType? in [.keyDown, .leftMouseUp, .rightMouseDown, .applicationDefined, nil] {
            for over: StatusItemController.Opener? in [.item, .label, nil] {
                let closing = Closing(at: 10, by: kind, eventTime: 10, over: over)
                #expect(!closing.wasMade(by: 10, on: .item), "\(String(describing: kind)), over \(String(describing: over))")
                #expect(!closing.wasMade(by: 10.05, on: .item))
                #expect(!closing.wasMade(by: 10.05, on: .label))
            }
        }
    }
}
