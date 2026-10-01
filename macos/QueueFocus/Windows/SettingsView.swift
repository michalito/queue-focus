import SwiftUI

/// Settings. Every change applies at once; the engine writes the file a
/// moment later. The flash, quiet hours and shortcuts join in Phase 4.
struct SettingsView: View {
    @Environment(QueueModel.self) private var model
    @AppStorage(Preferences.menuBarTitleWidthKey)
    private var titleWidth = Preferences.defaultMenuBarTitleWidth
    @Environment(LoginItem.self) private var loginItem

    var body: some View {
        Form {
            Section("Appearance") {
                Picker("Theme", selection: setting(\.theme)) {
                    Text("System").tag(Theme.system)
                    Text("Light").tag(Theme.light)
                    Text("Dark").tag(Theme.dark)
                }
            }
            Section("Menu bar") {
                Toggle("Show elapsed time", isOn: setting(\.showTimer))
                    .accessibilityIdentifier("setting-show-timer")
                LabeledContent("Title width") {
                    Slider(value: $titleWidth, in: Preferences.menuBarTitleWidthRange, step: 10)
                    Text("\(Int(titleWidth)) pt")
                        .monospacedDigit()
                        .frame(width: 52, alignment: .trailing)
                }
                .help("macOS hides menu bar items that do not fit, so a long title is cut here.")
            }
            Section("Adding") {
                Picker("Return adds to", selection: setting(\.defaultBucket)) {
                    Text("Next").tag(Bucket.next)
                    Text("Later").tag(Bucket.later)
                    Text("Side").tag(Bucket.side)
                    Text("Now").tag(Bucket.now)
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
        .frame(width: 440)
        .fixedSize(horizontal: false, vertical: true)
    }

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
}
