import AppKit
import SwiftUI

/// Settings' Sync group, as the GNOME app's: what sync is doing, and the
/// changes the server turned down.
struct SyncSettingsView: View {
    @Environment(QueueModel.self) private var model
    @State private var dismissing: SyncRecoveryChange?
    @State private var exported: URL?

    var body: some View {
        if let sync = model.snapshot.sync {
            SettingsGroup("Sync", description: "Now and the timer are shared. Queue edits can be saved offline.") {
                SettingsRow {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(model.syncMessage ?? "Sync enabled")
                            .foregroundStyle(Adw.dim)
                            .textSelection(.enabled)
                            .fixedSize(horizontal: false, vertical: true)
                        HStack(spacing: 8) {
                            Button("Retry connection") { model.retrySync() }
                                .disabled(sync.state == "recovery" || sync.state == "durability")
                            Button("Export recovery copy…", action: export)
                        }
                        .buttonStyle(.adwFilled)
                        if let exported {
                            Button("Show exported copy") { NSWorkspace.shared.activateFileViewerSelecting([exported]) }
                                .buttonStyle(.adwFilled)
                        }
                    }
                }
                if !model.recoveryChanges.isEmpty {
                    SettingsRow(divided: true) {
                        VStack(alignment: .leading, spacing: 12) {
                            RowLabel(title: "Rejected changes",
                                     subtitle: "These changes were not applied. Copy or export them before dismissing anything you still need.")
                            ForEach(model.recoveryChanges, id: \.command) { item in
                                VStack(alignment: .leading, spacing: 6) {
                                    Text(item.title).fontWeight(.medium)
                                    Text("\(item.attempt) · \(item.reason)")
                                        .foregroundStyle(Adw.dim)
                                    HStack(spacing: 8) {
                                        Button("Copy change") {
                                            NSPasteboard.general.clearContents()
                                            NSPasteboard.general.setString(item.copyText, forType: .string)
                                        }
                                        Button("Dismiss…", role: .destructive) { dismissing = item }
                                            .disabled(sync.state == "recovery")
                                    }
                                    .buttonStyle(.adwFilled)
                                }
                                .textSelection(.enabled)
                            }
                        }
                    }
                }
            }
            .confirmationDialog("Dismiss this rejected change?",
                                isPresented: Binding(get: { dismissing != nil },
                                                     set: { if !$0 { dismissing = nil } }),
                                titleVisibility: .visible) {
                if let item = dismissing {
                    Button("Dismiss saved change", role: .destructive) {
                        model.dismissRecovery(item.command)
                        dismissing = nil
                    }
                }
            } message: {
                Text("This removes the local recovery record. It does not change the shared queue or discard pending commands.")
            }
        } else {
            // Nothing to show but how to set it up.
            VStack(alignment: .leading, spacing: 2) {
                Text("Sync")
                    .font(Adw.font(1, .bold))
                    .accessibilityAddTraits(.isHeader)
                Text("Not configured. Quit Queue Focus and use the bundled qf-sync utility to preview and join your private server. Each device needs its own enrollment.")
                    .settingNote()
                    .font(Adw.font(0.85))
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.horizontal, 2)
            .accessibilityElement(children: .contain)
        }
    }

    private func export() {
        let panel = NSOpenPanel()
        panel.title = "Choose where to save a recovery copy"
        panel.message = "A new folder will contain your queue, settings and pending/rejected changes. The active installation stays intact."
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Export here"
        panel.begin { response in
            guard response == .OK, let parent = panel.url else { return }
            let destination = parent.appendingPathComponent("Queue Focus Recovery \(UUID().uuidString)")
            if model.exportSync(to: destination) { exported = destination }
        }
    }
}
