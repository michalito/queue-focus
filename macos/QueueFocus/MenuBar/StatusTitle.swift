import Foundation

/// What the status item says: a dot in the current task's tag colour, its
/// title cut to a width, and its clock. GNOME's top bar reads the same way.
struct StatusTitle: Equatable {
    /// Leads the clock while the timer is paused, as on the GNOME top bar.
    static let pauseGlyph = "❚❚"
    static let ellipsis = "…"
    /// What the title says while Now is empty.
    static let empty = "no task"

    /// The current task's tag, or `nil` for no tag or no task.
    var tag: TaskTag?
    /// The title as shown, cut to the width with an ellipsis if it had to be.
    var title: String
    /// The whole title, for the tooltip and VoiceOver.
    var fullTitle: String
    /// `"12m"`, or `"❚❚ 12m"` while paused; `nil` when hidden.
    var timer: String?
    var paused: Bool

    /// - Parameters:
    ///   - now: unix seconds.
    ///   - maxWidth: the most the title may take, in points.
    ///   - measure: the width of a string in the menu bar's font.
    static func make(
        current: QueueTask?,
        now: UInt64,
        showTimer: Bool,
        maxWidth: Double,
        measure: (String) -> Double
    ) -> StatusTitle {
        guard let task = current else {
            return StatusTitle(tag: nil, title: empty, fullTitle: empty, timer: nil, paused: false)
        }
        let paused = task.pausedAt != nil
        var timer: String?
        if showTimer, let secs = elapsedSecs(task: task, now: now) {
            let clock = shortElapsed(secs: secs, paused: false)
            timer = paused ? "\(pauseGlyph) \(clock)" : clock
        }
        return StatusTitle(
            tag: task.tag,
            title: cut(task.title, to: maxWidth, measure: measure),
            fullTitle: task.title,
            timer: timer,
            paused: paused
        )
    }

    /// The longest start of `text`, whole characters only, that fits in
    /// `width` with an ellipsis after it; `text` itself when it fits whole.
    static func cut(_ text: String, to width: Double, measure: (String) -> Double) -> String {
        guard measure(text) > width else { return text }
        let characters = Array(text)
        // The most characters that still fit, found by halving.
        var fits = 0
        var tooMany = characters.count
        while tooMany - fits > 1 {
            let middle = (fits + tooMany) / 2
            if measure(String(characters[..<middle]) + ellipsis) <= width {
                fits = middle
            } else {
                tooMany = middle
            }
        }
        let kept = String(characters[..<fits]).trimmingCharacters(in: .whitespaces)
        return kept + ellipsis
    }

    /// What VoiceOver reads for the status item.
    var accessibilityLabel: String {
        var parts = [fullTitle]
        if paused { parts.append("paused") }
        if let timer { parts.append(timer.replacingOccurrences(of: Self.pauseGlyph, with: "").trimmingCharacters(in: .whitespaces)) }
        return parts.joined(separator: ", ")
    }
}
