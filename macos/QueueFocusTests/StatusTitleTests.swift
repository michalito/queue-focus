import AppKit
import Testing
@testable import QueueFocus

/// Every character ten points wide, so widths are easy to reason about.
private func mono(_ text: String) -> Double {
    Double(text.count * 10)
}

private func task(_ title: String, tag: TaskTag? = nil, started: UInt64? = 0, paused: UInt64? = nil) -> QueueTask {
    QueueTask(id: 1, title: title, bucket: .now, tag: tag, createdAt: 0, startedAt: started, pausedAt: paused)
}

@Suite struct StatusTitleTests {
    @Test func anEmptyNowSaysSo() {
        let title = StatusTitle.make(current: nil, now: 100, showTimer: true, maxWidth: 200, measure: mono)
        #expect(title.title == "no task")
        #expect(title.timer == nil)
        #expect(title.tag == nil)
    }

    @Test func aTitleThatFitsIsShownWhole() {
        let title = StatusTitle.make(current: task("ship it", tag: .work), now: 12 * 60, showTimer: true, maxWidth: 200, measure: mono)
        #expect(title.title == "ship it")
        #expect(title.fullTitle == "ship it")
        #expect(title.tag == .work)
        #expect(title.timer == "12m")
    }

    @Test func aLongTitleIsCutToTheWidthWithAnEllipsis() {
        let long = "write the release notes for version two"
        let title = StatusTitle.make(current: task(long), now: 0, showTimer: false, maxWidth: 100, measure: mono)
        // Ten characters of ten points, the ellipsis one of them.
        #expect(title.title == "write the…")
        #expect(mono(title.title) <= 100)
        #expect(title.fullTitle == long)
    }

    /// A cut never splits a character a person sees as one.
    @Test func aCutKeepsWholeCharacters() {
        let family = "👩‍👩‍👧"
        let text = String(repeating: family, count: 20)
        let cut = StatusTitle.cut(text, to: 55, measure: mono)
        #expect(cut == String(repeating: family, count: 4) + "…")
        #expect(cut.unicodeScalars.count == 4 * family.unicodeScalars.count + 1)
    }

    @Test func aCutDropsTheSpaceBeforeTheEllipsis() {
        #expect(StatusTitle.cut("abcd efgh", to: 60, measure: mono) == "abcd…")
    }

    @Test func aCutWithNoRoomLeavesTheEllipsis() {
        #expect(StatusTitle.cut("abcdef", to: 5, measure: mono) == "…")
    }

    @Test func aPausedClockIsLedByTheGlyph() {
        let title = StatusTitle.make(current: task("t", started: 0, paused: 3_720), now: 9_999, showTimer: true, maxWidth: 200, measure: mono)
        #expect(title.timer == "❚❚ 1h02")
        #expect(title.paused)
        #expect(title.accessibilityLabel == "t, paused, 1h02")
    }

    /// The menu bar's clock is the core's: the same words GNOME's top bar shows.
    @Test func theClockIsTheCoresShortForm() {
        for secs: UInt64 in [0, 59, 60, 3_599, 3_600, 7_384, 360_000] {
            let title = StatusTitle.make(current: task("t", started: 0), now: secs, showTimer: true, maxWidth: 200, measure: mono)
            #expect(title.timer == shortElapsed(secs: secs, paused: false))
        }
    }

    @Test func theTimerSettingHidesTheClock() {
        let title = StatusTitle.make(current: task("t"), now: 600, showTimer: false, maxWidth: 200, measure: mono)
        #expect(title.timer == nil)
    }

    /// GNOME dims a paused task's label whether or not its clock shows; so
    /// does the menu bar, so a pause shows with the clock hidden too.
    @Test @MainActor func aPausedTitleIsDimmedWithOrWithoutItsClock() throws {
        let font = NSFont.menuBarFont(ofSize: 0)
        let dimmed = StatusItemController.pausedTitleColor
        for showTimer in [true, false] {
            for paused: UInt64? in [nil, 60] {
                let title = StatusTitle.make(current: task("ship", started: 0, paused: paused), now: 600, showTimer: showTimer,
                                             maxWidth: 200, measure: mono)
                let text = StatusItemController.render(title, font: font, options: DisplayOptions())
                let words = (text.string as NSString).range(of: "ship")
                let colour = text.attribute(.foregroundColor, at: words.location, effectiveRange: nil) as? NSColor
                if paused == nil {
                    #expect(colour == nil, "running, clock \(showTimer): the menu bar's own colour")
                } else {
                    #expect(colour == dimmed, "paused, clock \(showTimer): dimmed")
                }
            }
        }
    }
}
