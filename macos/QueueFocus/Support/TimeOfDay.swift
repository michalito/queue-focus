import Foundation

/// A time of day as a date picker edits it. A picker edits a moment, and on
/// the day the clocks change some times of day do not happen and others
/// happen twice; on 1 January 2001 in UTC every one happens once.
extension TimeOfDay {
    static let zone = TimeZone(identifier: "UTC")!

    static let calendar: Calendar = {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = zone
        return calendar
    }()

    var date: Date {
        Date(timeIntervalSinceReferenceDate: TimeInterval(Int(hour) * 3600 + Int(minute) * 60))
    }

    init(_ date: Date) {
        let parts = Self.calendar.dateComponents([.hour, .minute], from: date)
        self.init(hour: UInt8(parts.hour ?? 0), minute: UInt8(parts.minute ?? 0))
    }
}
