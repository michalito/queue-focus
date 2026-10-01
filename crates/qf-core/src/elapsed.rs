//! How the time on the current task's clock is written. Every surface that
//! shows it calls these, so the top bar, the flash card and the windows agree.

/// The top bar's clock, in whole minutes: `"12m"`, `"1h02"`, and `" ⏸"`
/// after it while paused. The flash card shows the same.
pub fn short_elapsed(secs: u64, paused: bool) -> String {
    let (h, m) = (secs / 3600, (secs % 3600) / 60);
    let t = if h > 0 {
        format!("{h}h{m:02}")
    } else {
        format!("{m}m")
    };
    if paused {
        format!("{t} ⏸")
    } else {
        t
    }
}

/// The windows' clock, to the second: `"00:00"`, `"1:02:05"`.
pub fn long_elapsed(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60;

    #[test]
    fn the_short_clock_counts_whole_minutes_and_marks_a_pause() {
        assert_eq!(short_elapsed(0, false), "0m");
        assert_eq!(short_elapsed(59, false), "0m");
        assert_eq!(short_elapsed(12 * MIN, false), "12m");
        assert_eq!(short_elapsed(3600, false), "1h00");
        assert_eq!(short_elapsed(3600 + 2 * MIN, false), "1h02");
        assert_eq!(short_elapsed(12 * MIN, true), "12m ⏸");
        assert_eq!(short_elapsed(100 * 3600 + 5 * MIN, false), "100h05");
    }

    #[test]
    fn the_long_clock_counts_seconds() {
        assert_eq!(long_elapsed(0), "00:00");
        assert_eq!(long_elapsed(65), "01:05");
        assert_eq!(long_elapsed(762), "12:42");
        assert_eq!(long_elapsed(3600), "1:00:00");
        assert_eq!(long_elapsed(3725), "1:02:05");
    }
}
