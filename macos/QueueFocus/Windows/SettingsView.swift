import KeyboardShortcuts
import SwiftUI

/// Settings: the reminder first, then the rules that keep it quiet, then the
/// small things. Every change applies at once; the engine writes the file a
/// moment later.
struct SettingsView: View {
    @Environment(QueueModel.self) private var model
    @Environment(LoginItem.self) private var loginItem
    @AppStorage(Preferences.menuBarTitleWidthKey)
    private var titleWidth = Preferences.defaultMenuBarTitleWidth

    var body: some View {
        VStack(spacing: 0) {
            form
            // A change that could not be saved says so here too: with only
            // Settings open, nowhere else would.
            MessageLine()
        }
        .frame(width: 480)
        .fixedSize(horizontal: false, vertical: true)
    }

    private var form: some View {
        Form {
            reminder
            quiet
            Section("Appearance") {
                Picker("Theme", selection: setting(\.theme)) {
                    Text("System").tag(Theme.system)
                    Text("Light").tag(Theme.light)
                    Text("Dark").tag(Theme.dark)
                }
                .accessibilityIdentifier("setting-theme")
            }
            Section("Menu bar") {
                Toggle(isOn: setting(\.showTimer)) {
                    Text("Show elapsed time")
                    Text("Off keeps the title alone: ● ship v0.1")
                }
                .accessibilityIdentifier("setting-show-timer")
                LabeledContent("Title width") {
                    Slider(value: $titleWidth, in: Preferences.menuBarTitleWidthRange, step: 10)
                    Text("\(Int(titleWidth)) pt")
                        .monospacedDigit()
                        .frame(width: 52, alignment: .trailing)
                }
                .help("macOS hides menu bar items that do not fit, so a long title is cut here.")
            }
            Section("Quick add") {
                Picker(selection: setting(\.defaultBucket)) {
                    Text("Next").tag(Bucket.next)
                    Text("Now").tag(Bucket.now)
                    Text("Side").tag(Bucket.side)
                    Text("Later").tag(Bucket.later)
                } label: {
                    Text("⏎ adds to")
                    Text("⌘⏎ always goes to Now; @markers still win.")
                }
                .accessibilityIdentifier("setting-default-bucket")
            }
            globalShortcuts
            Section("Keyboard") {
                LabeledContent("In the Queue and the Board") {
                    Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 3) {
                        ForEach(shortcutList, id: \.keys) { shortcut in
                            GridRow {
                                Text(shortcut.keys).font(.body.monospaced()).foregroundStyle(.secondary)
                                Text(shortcut.does)
                            }
                        }
                    }
                }
            }
            Section("General") {
                Toggle("Launch at login", isOn: Binding(get: { loginItem.isEnabled }, set: loginItem.set))
                if loginItem.needsApproval {
                    Button("Allow in System Settings…", action: loginItem.openSystemSettings)
                }
            }
        }
        .formStyle(.grouped)
    }

    // MARK: The reminder

    private var reminder: some View {
        Section {
            LabeledContent("Flash every") {
                Slider(value: interval, in: Double(intervalBounds().min)...Double(intervalBounds().max), step: 1) {
                    Text("Flash every, in minutes")
                } minimumValueLabel: {
                    Text("\(intervalBounds().min)m")
                } maximumValueLabel: {
                    Text("\(intervalBounds().max)")
                }
                .labelsHidden()
                .accessibilityIdentifier("setting-interval")
                Text("\(model.settings.intervalMin) min")
                    .monospacedDigit()
                    .frame(width: 56, alignment: .trailing)
            }
            Toggle(isOn: setting(\.vary)) {
                Text("Vary the flash")
                Text("Picks one of six styles at random, so you do not learn to ignore the one.")
            }
            Picker("Intensity", selection: setting(\.intensity)) {
                Text("subtle").tag(Intensity.subtle)
                Text("normal").tag(Intensity.normal)
                Text("strong").tag(Intensity.strong)
            }
            .pickerStyle(.segmented)
            Picker("Color", selection: setting(\.color)) {
                Text("follow tag").tag(FlashColor.tag)
                Text("blue").tag(FlashColor.blue)
                Text("orange").tag(FlashColor.orange)
            }
            .pickerStyle(.segmented)
            TryIt()
        } header: {
            Text("Reminder")
        } footer: {
            Text("The screen flashes the current task and its time. Never while Now is empty.")
        }
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

    // MARK: Global shortcuts

    private var globalShortcuts: some View {
        Section {
            ForEach(Hotkey.allCases, id: \.self) { hotkey in
                KeyboardShortcuts.Recorder(hotkey.title, name: hotkey.name)
                    .shortcutValidation { shortcut in
                        guard let other = Hotkey.holder(of: shortcut, besides: hotkey,
                                                        shortcuts: { KeyboardShortcuts.getShortcut(for: $0.name) })
                        else { return .allow }
                        return .disallow(reason: "“\(other.title)” uses it already.")
                    }
            }
            Button("Restore Defaults") {
                KeyboardShortcuts.reset(Hotkey.allCases.map(\.name))
            }
        } header: {
            Text("Global shortcuts")
        } footer: {
            Text("They work whatever app is in front. If one does nothing, another app may be using it.")
        }
    }

    // MARK: Quiet

    private var quiet: some View {
        Section {
            Toggle(isOn: setting(\.quietPaused)) {
                Text("While the timer is paused")
                Text("A paused task is a deliberate break.")
            }
            Toggle(isOn: setting(\.quietHours)) {
                Text("Outside hours")
                Text("A range that ends before it starts runs past midnight; the same start and end is the whole day.")
            }
            .accessibilityIdentifier("setting-quiet-hours")
            LabeledContent("Flash only between") {
                DatePicker("Flash from", selection: time(\.quietFrom), displayedComponents: .hourAndMinute)
                    .labelsHidden()
                    .accessibilityIdentifier("setting-quiet-from")
                Text("and")
                DatePicker("Flash until", selection: time(\.quietTo), displayedComponents: .hourAndMinute)
                    .labelsHidden()
                    .accessibilityIdentifier("setting-quiet-to")
            }
            // A time of day, not a moment: shown in a zone whose clocks never
            // change, so 02:30 is never skipped or doubled.
            .environment(\.timeZone, TimeOfDay.zone)
            .disabled(!model.settings.quietHours)
        } header: {
            Text("Quiet")
        } footer: {
            Text("When the reminder holds back.")
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

/// The time until the next flash, or why there will not be one, and a
/// button to see one now. Both are read every second.
private struct TryIt: View {
    @Environment(QueueModel.self) private var model

    var body: some View {
        // Read so the line moves with the clock.
        let _ = model.now
        let status = model.flashStatus()
        LabeledContent {
            Button("Flash now") { model.flashNow() }
                .disabled(status == .held(reason: .noCurrentTask))
                .accessibilityIdentifier("flash-now")
        } label: {
            Text("Try it")
            Text(Self.describe(status))
                .accessibilityIdentifier("flash-status")
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
