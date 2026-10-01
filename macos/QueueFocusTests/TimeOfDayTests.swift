import Foundation
import Testing
@testable import QueueFocus

@Suite struct TimeOfDayTests {
    @Test func everyTimeOfDayComesBackFromThePicker() {
        for minutes in 0..<(24 * 60) {
            let time = TimeOfDay(hour: UInt8(minutes / 60), minute: UInt8(minutes % 60))
            #expect(TimeOfDay(time.date) == time)
        }
    }

    /// 02:30 does not happen in New York on 8 March 2026; a picker showing
    /// it on that day would show 03:00, and save that.
    @Test func aTimeTheClocksSkipIsShownAsItIs() {
        #expect(TimeOfDay.zone.nextDaylightSavingTimeTransition(after: .distantPast) == nil,
                "the pickers' zone never changes its clocks")
        let parts = TimeOfDay.calendar.dateComponents([.hour, .minute], from: TimeOfDay(hour: 2, minute: 30).date)
        #expect(parts.hour == 2 && parts.minute == 30)
    }
}
