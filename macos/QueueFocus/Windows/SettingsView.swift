import KeyboardShortcuts
import SwiftUI

/// Settings, laid out as the GNOME app's Settings page: the reminder first,
/// then the rules that keep it quiet, then the small things, a card per
/// group. Sync leads, as there; the Mac's own groups come last. Every change
/// applies at once; the engine writes the file a moment later.
struct SettingsView: View {
    @Environment(QueueModel.self) private var model
    @Environment(LoginItem.self) private var loginItem
    @Environment(\.dismissWindow) private var dismissWindow
    @State private var showShortcuts = false
    @AppStorage(Preferences.menuBarTitleWidthKey)
    private var titleWidth = Preferences.defaultMenuBarTitleWidth
    @AppStorage(Preferences.menuBarTitleCentredKey)
    private var titleCentred = false

    var body: some View {
        let accent = AdwAccent.of(model.snapshot.current?.tag)
        VStack(spacing: 0) {
            HeaderBar {
                Text("Settings")
                    .font(Adw.font(1.1, .bold))
                    .accessibilityAddTraits(.isHeader)
            } end: {
                // Lit while Settings shows, as the gear it came from is;
                // pressed, it goes back.
                HeaderGlyphButton(glyph: "gearshape.fill", symbol: true, lit: true) {
                    dismissWindow(id: WindowID.settings)
                }
                .help("Close Settings (⌘W)")
                .accessibilityLabel("Close Settings")
                HeaderGlyphButton(glyph: "?") { showShortcuts.toggle() }
                    .help("Keyboard shortcuts")
                    .accessibilityLabel("Keyboard shortcuts")
                    .popover(isPresented: $showShortcuts, arrowEdge: .bottom) {
                        ShortcutLines(rowSpacing: 5)
                            .font(Adw.font(0.8))
                            .padding(.horizontal, 14)
                            .padding(.vertical, 12)
                    }
            }
            .background(accent.headerBar)
            ScrollView {
                VStack(alignment: .leading, spacing: 22) {
                    SyncSettingsView()
                    reminder
                    quiet
                    SettingsGroup("Appearance") {
                        ChoiceRow(title: "Theme", options: [(.system, "System"), (.light, "Light"), (.dark, "Dark")],
                                  selection: setting(\.theme), identifier: "setting-theme")
                    }
                    menuBar
                    SettingsGroup("Quick add") {
                        ChoiceRow(title: "⏎ adds to",
                                  subtitle: model.snapshot.sync == nil
                                      ? "To Now while Now is empty. ⌘⏎ always goes to Now; @markers still win."
                                      : "Uses this queue even while Now is empty. Now changes need a connection; @markers still win.",
                                  options: [(Bucket.next, "Next"), (.now, "Now"), (.side, "Side"), (.later, "Later")],
                                  selection: setting(\.defaultBucket), narrow: true, identifier: "setting-default-bucket")
                    }
                    globalShortcuts
                    SettingsGroup("Keyboard", description: "In the Queue and the Board.") {
                        SettingsRow {
                            ShortcutLines(rowSpacing: 4)
                                .font(Adw.font(0.85))
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }
                    SettingsGroup("General") {
                        SettingsRow {
                            Toggle(isOn: Binding(get: { loginItem.isEnabled }, set: { loginItem.set($0) })) {
                                RowLabel(title: "Launch at login")
                            }
                            .toggleStyle(AdwSwitchStyle())
                        }
                        if loginItem.needsApproval {
                            SettingsRow(divided: true) {
                                Button("Allow in System Settings…", action: loginItem.openSystemSettings)
                                    .buttonStyle(.adwFilled)
                            }
                        }
                    }
                }
                .padding(EdgeInsets(top: 18, leading: 20, bottom: 24, trailing: 20))
            }
            // A change that could not be saved says so here too: with only
            // Settings open, nowhere else would.
            MessageLine()
        }
        // The page scrolls: grown to fit every group, the window ran past
        // the bottom of a laptop's screen. As wide as GNOME's window.
        .frame(width: 400)
        .frame(minHeight: 420, idealHeight: 640, maxHeight: .infinity)
        .navigationTitle("Settings")
        .background(Adw.window)
        .foregroundStyle(Adw.foreground)
        .modifier(HeaderBarWindow())
        .environment(\.adwAccent, accent)
        .tint(accent.background)
    }

    // MARK: The reminder

    private var reminder: some View {
        SettingsGroup("Reminder", description: "The screen flashes the current task and its time. Never while Now is empty.") {
            IntervalRow()
            SettingsRow(divided: true) {
                Toggle(isOn: setting(\.vary)) {
                    RowLabel(title: "Vary the flash",
                             subtitle: "Picks one of six styles at random, so you do not learn to ignore the one.")
                }
                .toggleStyle(AdwSwitchStyle())
            }
            ChoiceRow(title: "Intensity", options: [(.subtle, "subtle"), (.normal, "normal"), (.strong, "strong")],
                      selection: setting(\.intensity))
            ChoiceRow(title: "Color", options: [(.tag, "follow tag"), (.blue, "blue"), (.orange, "orange")],
                      selection: setting(\.color))
            TryIt()
        }
    }

    // MARK: Quiet

    private var quiet: some View {
        SettingsGroup("Quiet", description: "When the reminder holds back.") {
            SettingsRow {
                Toggle(isOn: setting(\.quietPaused)) {
                    RowLabel(title: "While the timer is paused", subtitle: "A paused task is a deliberate break.")
                }
                .toggleStyle(AdwSwitchStyle())
            }
            SettingsRow(divided: true) {
                Toggle(isOn: setting(\.quietHours)) {
                    RowLabel(title: "Outside hours",
                             subtitle: "A range that ends before it starts runs past midnight; the same start and end is the whole day.")
                }
                .toggleStyle(AdwSwitchStyle())
                .accessibilityIdentifier("setting-quiet-hours")
            }
            // Shown only while the switch above is on, as in GNOME.
            if model.settings.quietHours {
                SettingsRow(divided: true) {
                    HStack(spacing: 8) {
                        Text("Flash only between")
                            .foregroundStyle(Adw.foreground.opacity(0.75))
                            .frame(maxWidth: .infinity, alignment: .leading)
                        DatePicker("Flash from", selection: time(\.quietFrom), displayedComponents: .hourAndMinute)
                            .labelsHidden()
                            .accessibilityIdentifier("setting-quiet-from")
                        Text("–").foregroundStyle(Adw.foreground.opacity(0.75))
                        DatePicker("Flash until", selection: time(\.quietTo), displayedComponents: .hourAndMinute)
                            .labelsHidden()
                            .accessibilityIdentifier("setting-quiet-to")
                    }
                    .datePickerStyle(.field)
                    // A time of day, not a moment: shown in a zone whose
                    // clocks never change, so 02:30 is never skipped or
                    // doubled.
                    .environment(\.timeZone, TimeOfDay.zone)
                }
            }
        }
    }

    // MARK: The menu bar

    private var menuBar: some View {
        SettingsGroup("Menu bar") {
            SettingsRow {
                Toggle(isOn: setting(\.showTimer)) {
                    RowLabel(title: "Show elapsed time", subtitle: "Off keeps the title alone: ● ship v0.1")
                }
                .toggleStyle(AdwSwitchStyle())
                .accessibilityIdentifier("setting-show-timer")
            }
            SettingsRow(divided: true) {
                VStack(spacing: 8) {
                    HStack {
                        Text("Title width")
                        Spacer()
                        ValueLabel(text: "\(Int(titleWidth)) pt")
                    }
                    AdwSlider(value: $titleWidth, range: Preferences.menuBarTitleWidthRange, step: 10,
                              label: "Title width", valueDescription: "\(Int(titleWidth)) points")
                }
            }
            .help("macOS hides menu bar items that do not fit, so a long title is cut here.")
            SettingsRow(divided: true) {
                Toggle(isOn: $titleCentred) {
                    RowLabel(title: "Task in the centre of the menu bar",
                             subtitle: "The item keeps a dot. Other apps' menus and icons are not seen: on a full menu bar the title can cover them.")
                }
                .toggleStyle(AdwSwitchStyle())
                .accessibilityIdentifier("setting-title-centred")
            }
        }
    }

    // MARK: Global shortcuts

    private var globalShortcuts: some View {
        SettingsGroup("Global shortcuts",
                      description: "They work whatever app is in front. If one does nothing, another app may be using it.") {
            ForEach(Array(Hotkey.allCases.enumerated()), id: \.element) { index, hotkey in
                SettingsRow(divided: index > 0) {
                    HStack(spacing: 12) {
                        Text(hotkey.title)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        KeyboardShortcuts.Recorder(for: hotkey.name)
                            .shortcutValidation { shortcut in
                                guard let other = Hotkey.holder(of: shortcut, besides: hotkey,
                                                                shortcuts: { KeyboardShortcuts.getShortcut(for: $0.name) })
                                else { return .allow }
                                return .disallow(reason: "“\(other.title)” uses it already.")
                            }
                            .fixedSize()
                            // Named for VoiceOver, as the label beside it
                            // names it for the eye.
                            .accessibilityLabel(hotkey.title)
                    }
                }
            }
            SettingsRow(divided: true) {
                HStack {
                    Spacer()
                    Button("Restore Defaults") {
                        KeyboardShortcuts.reset(Hotkey.allCases.map(\.name))
                    }
                    .font(Adw.font(1, .bold))
                    .buttonStyle(.adwFilled)
                }
            }
        }
    }

    // MARK: Bindings

    /// A binding to one shared setting, written through the engine.
    private func setting<Value>(_ key: WritableKeyPath<QueueSettings, Value>) -> Binding<Value> {
        Binding(
            get: { model.settings[keyPath: key] },
            set: { value in
                var settings = model.settings
                settings[keyPath: key] = value
                model.setSettings(settings)
            }
        )
    }

    /// A time-of-day setting as the date a picker edits.
    private func time(_ key: WritableKeyPath<QueueSettings, TimeOfDay>) -> Binding<Date> {
        Binding(
            get: { model.settings[keyPath: key].date },
            set: { date in
                var settings = model.settings
                settings[keyPath: key] = TimeOfDay(date)
                model.setSettings(settings)
            }
        )
    }
}

// MARK: Groups and rows

/// A heading, an optional line of explanation, and the card the rows go in.
struct SettingsGroup<Rows: View>: View {
    let title: String
    var description: String?
    @ViewBuilder var rows: Rows

    init(_ title: String, description: String? = nil, @ViewBuilder rows: () -> Rows) {
        self.title = title
        self.description = description
        self.rows = rows()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(Adw.font(1, .bold))
                    .accessibilityAddTraits(.isHeader)
                if let description {
                    Text(description).settingNote()
                        .font(Adw.font(0.85))
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .padding(.horizontal, 2)
            VStack(alignment: .leading, spacing: 0) {
                rows
            }
            .adwCard()
        }
        .accessibilityElement(children: .contain)
    }
}

/// One row of a card, under a hairline unless it is the first.
struct SettingsRow<Content: View>: View {
    var divided = false
    var padding = EdgeInsets(top: 10, leading: 14, bottom: 10, trailing: 14)
    @ViewBuilder var content: Content

    var body: some View {
        VStack(spacing: 0) {
            if divided {
                Hairline()
            }
            content
                .padding(padding)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

/// A row's name, and a line explaining it.
struct RowLabel: View {
    let title: String
    var subtitle: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(title)
            if let subtitle {
                Text(subtitle).settingNote()
                    .font(Adw.font(0.85))
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// A row whose control is a segmented choice.
private struct ChoiceRow<Value: Hashable>: View {
    let title: String
    var subtitle: String?
    let options: [(Value, String)]
    let selection: Binding<Value>
    /// Four choices only fit across the window at a tighter padding.
    var narrow = false
    var identifier = ""

    var body: some View {
        SettingsRow(divided: true) {
            HStack(spacing: 12) {
                RowLabel(title: title, subtitle: subtitle)
                Segmented(options: options.map { (value: $0.0, label: $0.1) }, selection: selection.wrappedValue,
                          horizontalPadding: narrow ? 8 : 12) { selection.wrappedValue = $0 }
                    .accessibilityLabel(title)
                    .accessibilityIdentifier(identifier)
            }
        }
    }
}

/// A setting's value beside its name, in the accent's monospace.
private struct ValueLabel: View {
    let text: String
    @Environment(\.adwAccent) private var accent

    var body: some View {
        Text(text)
            .font(Adw.font(1, .semibold, design: .monospaced))
            .monospacedDigit()
            .foregroundStyle(accent.ink)
    }
}

/// "Flash every": the value, the slider, and the marks along it.
private struct IntervalRow: View {
    @Environment(QueueModel.self) private var model
    private static let marks: [(Double, String)] = [(15, "15"), (30, "30"), (60, "60")]

    var body: some View {
        let bounds = intervalBounds()
        let range = Double(bounds.min)...Double(bounds.max)
        // The marks hang below the slider, so the row needs less room under.
        SettingsRow(padding: EdgeInsets(top: 10, leading: 14, bottom: 4, trailing: 14)) {
            VStack(spacing: 4) {
                HStack {
                    Text("Flash every")
                    Spacer()
                    ValueLabel(text: "\(model.settings.intervalMin) min")
                }
                AdwSlider(value: interval, range: range, step: 1, label: "Flash every",
                          valueDescription: "\(model.settings.intervalMin) minutes")
                    .accessibilityIdentifier("setting-interval")
                marksRow(range: range, bounds: bounds)
            }
        }
    }

    /// GTK's scale marks: a tick and a number under each.
    private func marksRow(range: ClosedRange<Double>, bounds: IntervalBounds) -> some View {
        let marks = [(range.lowerBound, "\(bounds.min)m")] + Self.marks + [(range.upperBound, "\(bounds.max)")]
        return GeometryReader { geometry in
            // The knob stops half its width in from either end.
            let inset = AdwSlider.knob / 2
            let span = geometry.size.width - 2 * inset
            ForEach(marks, id: \.1) { value, label in
                VStack(spacing: 1) {
                    Rectangle().fill(Adw.dim).frame(width: 1, height: 4)
                    Text(label).font(Adw.font(0.75)).foregroundStyle(Adw.dim)
                        .fixedSize()
                }
                .position(x: inset + span * (value - range.lowerBound) / (range.upperBound - range.lowerBound), y: 9)
            }
        }
        .frame(height: 20)
        .accessibilityHidden(true)
    }

    private var interval: Binding<Double> {
        Binding(
            get: { Double(model.settings.intervalMin) },
            set: { minutes in
                var settings = model.settings
                settings.intervalMin = UInt32(minutes.rounded())
                model.setSettings(settings)
            }
        )
    }
}

/// The time until the next flash, or why there will not be one, and a
/// button to see one now. Both are read every second.
private struct TryIt: View {
    @Environment(QueueModel.self) private var model

    var body: some View {
        // Read so the line moves with the clock.
        let _ = model.now
        let status = model.flashStatus()
        SettingsRow(divided: true) {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 1) {
                    Text("Try it")
                    Text(Self.describe(status))
                        .font(Adw.font(0.85))
                        .foregroundStyle(Adw.dim)
                        .accessibilityIdentifier("flash-status")
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                Button("Flash now") { model.flashNow() }
                    .font(Adw.font(1, .bold))
                    .buttonStyle(AdwButtonStyle(kind: .filled, padding: EdgeInsets(top: 7, leading: 17, bottom: 7, trailing: 17)))
                    .disabled(status == .held(reason: .noCurrentTask))
                    .accessibilityIdentifier("flash-now")
            }
        }
    }

    static func describe(_ status: FlashStatus?) -> String {
        switch status {
        case .scheduled(let remaining)?: "Next flash in \(longElapsed(secs: remaining))"
        case .held(let reason)?: "No flash: \(holdReasonText(reason: reason))"
        case nil: ""
        }
    }
}

extension Text {
    /// A line that explains a setting: grey, but a grey that reads, where
    /// GNOME's faded text measures under 4.5 to 1. Text only: the
    /// accessibility audit's verdict on a note is checked against its
    /// pixels, which holds only for text of one colour.
    func settingNote() -> some View {
        foregroundStyle(Adw.dim).accessibilityIdentifier("setting-note")
    }
}
