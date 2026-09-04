//! User settings: the flash reminder, the quiet rules, and the few defaults
//! the app and the shell extension have to agree on.
//!
//! Everything here is pure: no clock, no randomness, no files. The caller
//! supplies the local time of day and a random number, so the decisions the
//! reminder makes are ordinary functions with ordinary tests.

use crate::{Bucket, Task};
use serde::de::{self, Deserializer, Unexpected};
use serde::{Deserialize, Serialize, Serializer};
use std::fmt;

/// Bounds of the "flash every" slider, in minutes.
pub const INTERVAL_MIN: u32 = 1;
pub const INTERVAL_MAX: u32 = 90;

/// How loud a flash is. The `Settings` control picks one for every flash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Intensity {
    Subtle,
    #[default]
    Normal,
    Strong,
}

impl Intensity {
    pub const ALL: [Intensity; 3] = [Intensity::Subtle, Intensity::Normal, Intensity::Strong];

    /// The wire name and the button label are the same word here.
    pub fn label(self) -> &'static str {
        self.as_str()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Intensity::Subtle => "subtle",
            Intensity::Normal => "normal",
            Intensity::Strong => "strong",
        }
    }
}

/// Which colour a flash is painted in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlashColor {
    /// Follow the current task's tag; untagged flashes blue.
    #[default]
    Tag,
    Blue,
    Orange,
}

impl FlashColor {
    pub const ALL: [FlashColor; 3] = [FlashColor::Tag, FlashColor::Blue, FlashColor::Orange];

    pub fn as_str(self) -> &'static str {
        match self {
            FlashColor::Tag => "tag",
            FlashColor::Blue => "blue",
            FlashColor::Orange => "orange",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FlashColor::Tag => "follow tag",
            FlashColor::Blue => "blue",
            FlashColor::Orange => "orange",
        }
    }

    /// The palette a flash is drawn in: the tag whose colours to use, or the
    /// untagged default. Both ends of the wire agree on the two names.
    pub fn palette(self, tag: Option<crate::Tag>) -> Palette {
        match self {
            FlashColor::Blue => Palette::Blue,
            FlashColor::Orange => Palette::Orange,
            FlashColor::Tag => match tag {
                Some(crate::Tag::Personal) => Palette::Orange,
                _ => Palette::Blue,
            },
        }
    }
}

/// The resolved colour of one flash. The extension owns the hex values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Palette {
    Blue,
    Orange,
}

impl Palette {
    pub fn as_str(self) -> &'static str {
        match self {
            Palette::Blue => "blue",
            Palette::Orange => "orange",
        }
    }
}

/// Window colour scheme. `System` follows the desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::System, Theme::Light, Theme::Dark];

    pub fn as_str(self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Theme::System => "System",
            Theme::Light => "Light",
            Theme::Dark => "Dark",
        }
    }
}

/// One of the six ways the screen can flash. Three families, two each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlashStyle {
    /// Accent tint over the whole screen, one fade.
    Wash,
    /// The same tint, two beats.
    Wash2,
    /// Accent frame and inward glow, pulsing twice.
    Edges,
    /// One slow soft breath of the same glow, without the frame.
    EdgesSoft,
    /// The top bar goes accent.
    Topbar,
    /// The top bar goes accent and a beam drops to the card.
    TopbarBeam,
}

impl FlashStyle {
    pub const ALL: [FlashStyle; 6] = [
        FlashStyle::Wash,
        FlashStyle::Wash2,
        FlashStyle::Edges,
        FlashStyle::EdgesSoft,
        FlashStyle::Topbar,
        FlashStyle::TopbarBeam,
    ];

    /// The style used when "vary the flash" is off.
    pub const FIXED: FlashStyle = FlashStyle::Edges;

    pub fn as_str(self) -> &'static str {
        match self {
            FlashStyle::Wash => "wash",
            FlashStyle::Wash2 => "wash2",
            FlashStyle::Edges => "edges",
            FlashStyle::EdgesSoft => "edgesSoft",
            FlashStyle::Topbar => "topbar",
            FlashStyle::TopbarBeam => "topbarBeam",
        }
    }
}

/// Choose the next style. `vary` off always gives the same one; vary on picks
/// uniformly from the styles that are not `last`, so the same flash never
/// arrives twice in a row and never becomes furniture.
///
/// `random` is any number: the caller owns the entropy so this stays testable.
pub fn pick_style(vary: bool, last: Option<FlashStyle>, random: u32) -> FlashStyle {
    if !vary {
        return FlashStyle::FIXED;
    }
    let pool: Vec<FlashStyle> = FlashStyle::ALL
        .into_iter()
        .filter(|s| Some(*s) != last)
        .collect();
    pool[random as usize % pool.len()]
}

/// A time of day, as minutes since local midnight. Written as `"09:00"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimeOfDay(u16);

impl TimeOfDay {
    pub fn new(hour: u32, minute: u32) -> Option<TimeOfDay> {
        if hour > 23 || minute > 59 {
            return None;
        }
        Some(TimeOfDay(hour as u16 * 60 + minute as u16))
    }

    pub fn hour(self) -> u16 {
        self.0 / 60
    }

    pub fn minute(self) -> u16 {
        self.0 % 60
    }

    /// `"9:00"` and `"09:00"` both parse; anything else does not.
    pub fn parse(s: &str) -> Option<TimeOfDay> {
        let (hour, minute) = s.trim().split_once(':')?;
        let hour: u32 = hour.trim().parse().ok()?;
        let minute: u32 = minute.trim().parse().ok()?;
        TimeOfDay::new(hour, minute)
    }
}

impl fmt::Display for TimeOfDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour(), self.minute())
    }
}

impl Serialize for TimeOfDay {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for TimeOfDay {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<TimeOfDay, D::Error> {
        let raw = String::deserialize(deserializer)?;
        TimeOfDay::parse(&raw).ok_or_else(|| {
            de::Error::invalid_value(Unexpected::Str(&raw), &"a time like \"09:00\"")
        })
    }
}

/// Is `now` inside the daily window `[from, to)`? A window whose end is not
/// after its start wraps past midnight (`22:00`–`06:00`), and a window with
/// both ends the same is the whole day rather than none of it — a reminder
/// should never be silenced by a setting that reads like "all day".
pub fn within_window(now: TimeOfDay, from: TimeOfDay, to: TimeOfDay) -> bool {
    match from.cmp(&to) {
        std::cmp::Ordering::Equal => true,
        std::cmp::Ordering::Less => now >= from && now < to,
        std::cmp::Ordering::Greater => now >= from || now < to,
    }
}

/// Why the reminder is holding back, or that it is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hold {
    /// Nothing is stopping the next flash.
    None,
    /// Now is empty: there is nothing to be reminded of.
    NoCurrentTask,
    /// The current task's timer is paused — a deliberate break.
    Paused,
    /// The time of day is outside the hours the user chose.
    OutsideHours,
}

impl Hold {
    pub fn is_none(self) -> bool {
        self == Hold::None
    }

    /// Shown under "Try it" in Settings, in place of the countdown.
    pub fn reason(self) -> &'static str {
        match self {
            Hold::None => "",
            Hold::NoCurrentTask => "nothing in Now",
            Hold::Paused => "the timer is paused",
            Hold::OutsideHours => "outside the chosen hours",
        }
    }
}

/// Everything the user can change. Field names are the JSON keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Minutes between flashes, within `INTERVAL_MIN..=INTERVAL_MAX`.
    pub interval_min: u32,
    /// Pick a different style each time rather than always `FlashStyle::FIXED`.
    pub vary: bool,
    pub intensity: Intensity,
    pub color: FlashColor,
    /// Hold flashes back while the current task's timer is paused.
    pub quiet_paused: bool,
    /// Only flash between `quiet_from` and `quiet_to`.
    pub quiet_hours: bool,
    pub quiet_from: TimeOfDay,
    pub quiet_to: TimeOfDay,
    pub theme: Theme,
    /// Show the elapsed time beside the title in the top bar.
    pub show_timer: bool,
    /// Where Enter in the quick-add entry puts a task without an `@marker`.
    pub default_bucket: Bucket,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            interval_min: 15,
            vary: true,
            intensity: Intensity::default(),
            color: FlashColor::default(),
            quiet_paused: true,
            quiet_hours: false,
            quiet_from: TimeOfDay::new(9, 0).expect("09:00"),
            quiet_to: TimeOfDay::new(18, 0).expect("18:00"),
            theme: Theme::default(),
            show_timer: true,
            default_bucket: Bucket::Next,
        }
    }
}

impl Settings {
    /// Pull values a hand-edited file could hold back into range.
    pub fn sanitize(&mut self) {
        self.interval_min = self.interval_min.clamp(INTERVAL_MIN, INTERVAL_MAX);
    }

    pub fn interval_secs(&self) -> u64 {
        self.interval_min.clamp(INTERVAL_MIN, INTERVAL_MAX) as u64 * 60
    }

    /// Why the next flash is being held back. `current` is the head of Now and
    /// `now` is the local time of day.
    pub fn hold(&self, current: Option<&Task>, now: TimeOfDay) -> Hold {
        let Some(task) = current else {
            return Hold::NoCurrentTask;
        };
        if self.quiet_paused && task.is_paused() {
            return Hold::Paused;
        }
        if self.quiet_hours && !within_window(now, self.quiet_from, self.quiet_to) {
            return Hold::OutsideHours;
        }
        Hold::None
    }

    /// The JSON the extension reads.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }

    /// Apply a JSON object of changed keys, leaving the rest alone. Returns
    /// the fields that could not be understood, so a caller can complain
    /// rather than silently ignoring a typo.
    pub fn apply_patch(&mut self, patch: &str) -> Result<(), String> {
        let patch: serde_json::Value =
            serde_json::from_str(patch).map_err(|e| format!("settings patch is not JSON: {e}"))?;
        let serde_json::Value::Object(patch) = patch else {
            return Err("settings patch is not a JSON object".into());
        };
        let mut merged = serde_json::to_value(&*self).map_err(|e| e.to_string())?;
        let serde_json::Value::Object(fields) = &mut merged else {
            return Err("settings did not serialise to an object".into());
        };
        for (key, value) in patch {
            if !fields.contains_key(&key) {
                return Err(format!("unknown setting: {key}"));
            }
            fields.insert(key, value);
        }
        let mut next: Settings =
            serde_json::from_value(merged).map_err(|e| format!("bad setting value: {e}"))?;
        next.sanitize();
        *self = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Store, Tag};

    fn at(hour: u32, minute: u32) -> TimeOfDay {
        TimeOfDay::new(hour, minute).unwrap()
    }

    #[test]
    fn defaults_match_the_design() {
        let s = Settings::default();
        assert_eq!(s.interval_min, 15);
        assert!(s.vary);
        assert_eq!(s.intensity, Intensity::Normal);
        assert_eq!(s.color, FlashColor::Tag);
        assert!(s.quiet_paused);
        assert!(!s.quiet_hours);
        assert_eq!(s.quiet_from, at(9, 0));
        assert_eq!(s.quiet_to, at(18, 0));
        assert_eq!(s.theme, Theme::System);
        assert!(s.show_timer);
        assert_eq!(s.default_bucket, Bucket::Next);
    }

    #[test]
    fn time_of_day_reads_and_writes_a_clock_face() {
        assert_eq!(at(9, 0).to_string(), "09:00");
        assert_eq!(at(23, 59).to_string(), "23:59");
        assert_eq!(TimeOfDay::parse("9:05"), Some(at(9, 5)));
        assert_eq!(TimeOfDay::parse(" 09:05 "), Some(at(9, 5)));
        assert_eq!(TimeOfDay::parse("24:00"), None);
        assert_eq!(TimeOfDay::parse("09:60"), None);
        assert_eq!(TimeOfDay::parse("0900"), None);
        assert_eq!(TimeOfDay::parse(""), None);
        assert_eq!(TimeOfDay::parse("09:-1"), None);
    }

    #[test]
    fn a_window_covers_its_start_and_stops_before_its_end() {
        let (from, to) = (at(9, 0), at(18, 0));
        assert!(!within_window(at(8, 59), from, to));
        assert!(within_window(at(9, 0), from, to));
        assert!(within_window(at(17, 59), from, to));
        assert!(!within_window(at(18, 0), from, to), "the end is not inside");
    }

    #[test]
    fn a_window_that_ends_before_it_starts_wraps_past_midnight() {
        let (from, to) = (at(22, 0), at(6, 0));
        assert!(within_window(at(22, 0), from, to));
        assert!(within_window(at(23, 59), from, to));
        assert!(within_window(at(0, 0), from, to));
        assert!(within_window(at(5, 59), from, to));
        assert!(!within_window(at(6, 0), from, to));
        assert!(!within_window(at(12, 0), from, to));
    }

    /// Sliding both ends together must not silence the reminder for good.
    #[test]
    fn a_window_with_both_ends_the_same_is_the_whole_day() {
        let both = at(9, 0);
        for hour in 0..24 {
            assert!(within_window(at(hour, 30), both, both), "{hour}:30");
        }
    }

    #[test]
    fn nothing_in_now_holds_every_flash_back() {
        let s = Settings::default();
        assert_eq!(s.hold(None, at(12, 0)), Hold::NoCurrentTask);
        assert_eq!(Hold::NoCurrentTask.reason(), "nothing in Now");
    }

    #[test]
    fn a_paused_timer_holds_flashes_back_only_when_asked() {
        let mut store = Store::new();
        store.add("a", Bucket::Now, None, false);
        assert!(store.toggle_pause());
        let mut s = Settings::default();

        assert_eq!(s.hold(store.current(), at(12, 0)), Hold::Paused);
        s.quiet_paused = false;
        assert_eq!(s.hold(store.current(), at(12, 0)), Hold::None);
    }

    #[test]
    fn quiet_hours_apply_only_when_switched_on() {
        let mut store = Store::new();
        store.add("a", Bucket::Now, None, false);
        let mut s = Settings::default();

        assert_eq!(s.hold(store.current(), at(3, 0)), Hold::None);
        s.quiet_hours = true;
        assert_eq!(s.hold(store.current(), at(3, 0)), Hold::OutsideHours);
        assert_eq!(s.hold(store.current(), at(9, 0)), Hold::None);
    }

    /// An empty Now is reported before the quieter reasons: it is the one the
    /// user can act on.
    #[test]
    fn an_empty_now_outranks_the_quiet_rules() {
        let s = Settings {
            quiet_hours: true,
            ..Settings::default()
        };
        assert_eq!(s.hold(None, at(3, 0)), Hold::NoCurrentTask);
    }

    #[test]
    fn a_fixed_flash_is_always_the_same_one() {
        for random in 0..12 {
            assert_eq!(pick_style(false, None, random), FlashStyle::Edges);
            assert_eq!(
                pick_style(false, Some(FlashStyle::Edges), random),
                FlashStyle::Edges
            );
        }
    }

    #[test]
    fn a_varied_flash_never_repeats_the_one_before_it_and_reaches_them_all() {
        for last in FlashStyle::ALL {
            let mut seen = Vec::new();
            for random in 0..5 {
                let style = pick_style(true, Some(last), random);
                assert_ne!(style, last, "repeated {}", last.as_str());
                seen.push(style);
            }
            assert_eq!(seen.len(), 5);
            for style in FlashStyle::ALL.into_iter().filter(|s| *s != last) {
                assert!(seen.contains(&style), "{} unreachable", style.as_str());
            }
        }
        // With nothing to avoid, all six are reachable.
        let first: Vec<FlashStyle> = (0..6).map(|r| pick_style(true, None, r)).collect();
        for style in FlashStyle::ALL {
            assert!(first.contains(&style), "{} unreachable", style.as_str());
        }
    }

    #[test]
    fn the_palette_follows_the_tag_unless_a_colour_was_chosen() {
        assert_eq!(FlashColor::Tag.palette(None), Palette::Blue);
        assert_eq!(FlashColor::Tag.palette(Some(Tag::Work)), Palette::Blue);
        assert_eq!(
            FlashColor::Tag.palette(Some(Tag::Personal)),
            Palette::Orange
        );
        assert_eq!(FlashColor::Blue.palette(Some(Tag::Personal)), Palette::Blue);
        assert_eq!(FlashColor::Orange.palette(Some(Tag::Work)), Palette::Orange);
    }

    /// The extension switches on these exact strings, and settings.json holds
    /// them. `as_str` and serde have to agree, in both directions.
    #[test]
    fn wire_names_are_the_ones_both_ends_agree_on() {
        macro_rules! wire {
            ($ty:ty, $all:expr) => {
                for value in $all {
                    let json = serde_json::to_string(&value).unwrap();
                    assert_eq!(json, format!("\"{}\"", value.as_str()));
                    assert_eq!(serde_json::from_str::<$ty>(&json).unwrap(), value);
                }
            };
        }
        wire!(FlashStyle, FlashStyle::ALL);
        wire!(Intensity, Intensity::ALL);
        wire!(FlashColor, FlashColor::ALL);
        wire!(Theme, Theme::ALL);

        // The six style names, spelled out once so a rename has to be deliberate.
        let names: Vec<&str> = FlashStyle::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            names,
            [
                "wash",
                "wash2",
                "edges",
                "edgesSoft",
                "topbar",
                "topbarBeam"
            ]
        );
        assert!(serde_json::from_str::<FlashStyle>("\"nonsense\"").is_err());
    }

    #[test]
    fn a_patch_naming_several_keys_is_refused_whole_when_one_is_wrong() {
        let mut s = Settings::default();
        let before = s.clone();
        assert!(s
            .apply_patch(r#"{"intensity":"strong","vary":false,"theme":"puce"}"#)
            .is_err());
        assert_eq!(s, before, "the good keys did not slip through");
    }

    #[test]
    fn settings_round_trip_through_json() {
        let s = Settings {
            interval_min: 42,
            color: FlashColor::Orange,
            quiet_hours: true,
            quiet_from: at(22, 0),
            quiet_to: at(6, 30),
            theme: Theme::Dark,
            default_bucket: Bucket::Side,
            ..Settings::default()
        };

        let json = s.to_json();
        assert!(json.contains("\"quiet_from\":\"22:00\""), "{json}");
        assert!(json.contains("\"quiet_to\":\"06:30\""), "{json}");
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), s);
    }

    #[test]
    fn a_file_from_an_older_version_keeps_the_defaults_it_does_not_mention() {
        let s: Settings = serde_json::from_str(r#"{"interval_min":5,"vary":false}"#).unwrap();
        assert_eq!(s.interval_min, 5);
        assert!(!s.vary);
        assert_eq!(s.theme, Theme::System);
        assert_eq!(s.quiet_from, at(9, 0));
        // A key from a newer version is ignored rather than fatal.
        let s: Settings = serde_json::from_str(r#"{"interval_min":7,"whats_this":1}"#).unwrap();
        assert_eq!(s.interval_min, 7);
    }

    #[test]
    fn a_hand_edited_interval_is_pulled_back_into_range() {
        let mut s: Settings = serde_json::from_str(r#"{"interval_min":0}"#).unwrap();
        s.sanitize();
        assert_eq!(s.interval_min, INTERVAL_MIN);
        let mut s: Settings = serde_json::from_str(r#"{"interval_min":100000}"#).unwrap();
        s.sanitize();
        assert_eq!(s.interval_min, INTERVAL_MAX);
        assert_eq!(s.interval_secs(), INTERVAL_MAX as u64 * 60);
    }

    #[test]
    fn a_patch_changes_only_the_keys_it_names() {
        let mut s = Settings::default();
        s.apply_patch(r#"{"intensity":"strong","interval_min":3}"#)
            .unwrap();
        assert_eq!(s.intensity, Intensity::Strong);
        assert_eq!(s.interval_min, 3);
        assert!(s.vary, "untouched");
        assert_eq!(s.quiet_to, at(18, 0), "untouched");
    }

    #[test]
    fn a_patch_is_refused_whole_when_any_of_it_is_wrong() {
        let mut s = Settings::default();
        for bad in [
            r#"{"intensity":"loud"}"#,
            r#"{"quiet_from":"9am"}"#,
            r#"{"nonsense":true}"#,
            r#"{"vary":"yes"}"#,
            r#"["intensity","strong"]"#,
            r#"not json"#,
        ] {
            assert!(s.apply_patch(bad).is_err(), "{bad}");
            assert_eq!(s, Settings::default(), "{bad} changed something");
        }
    }

    #[test]
    fn a_patched_interval_is_clamped_like_a_loaded_one() {
        let mut s = Settings::default();
        s.apply_patch(r#"{"interval_min":9000}"#).unwrap();
        assert_eq!(s.interval_min, INTERVAL_MAX);
    }
}
