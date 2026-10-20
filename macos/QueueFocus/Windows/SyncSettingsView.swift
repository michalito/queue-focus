import AppKit
import SwiftUI

struct SyncSettingsView: View {
    @Environment(QueueModel.self) private var model
    @State private var dismissing: SyncRecoveryChange?
    @State private var exported: URL?

    var body: some View {
        Section("Sync") {
            if let sync = model.snapshot.sync {
                Text(model.syncMessage ?? "Sync enabled")
                    .textSelection(.enabled)
                Text("Now and the timer are shared. Queue edits can be saved offline.")
                    .foregroundStyle(Color.readableGrey)
                HStack {
                    Button("Retry connection") { model.retrySync() }
                        .disabled(sync.state == "recovery" || sync.state == "durability")
                    Button("Export recovery copy…", action: export)
                }
                if let exported {
                    Button("Show exported copy") { NSWorkspace.shared.activateFileViewerSelecting([exported]) }
                }
                if !model.recoveryChanges.isEmpty {
                    Text("Rejected changes")
                        .font(.headline)
                    Text("These changes were not applied. Copy or export them before dismissing anything you still need.")
                        .foregroundStyle(Color.readableGrey)
                    ForEach(model.recoveryChanges, id: \.command) { item in
                        VStack(alignment: .leading, spacing: 6) {
                            Text(item.title).fontWeight(.medium)
                            Text("\(item.attempt) · \(item.reason)")
                                .foregroundStyle(Color.readableGrey)
                            HStack {
                                Button("Copy change") {
                                    NSPasteboard.general.clearContents()
                                    NSPasteboard.general.setString(item.copyText, forType: .string)
                                }
                                Button("Dismiss…", role: .destructive) { dismissing = item }
                                    .disabled(sync.state == "recovery")
                            }
                        }
                        .textSelection(.enabled)
                    }
                }
            } else {
                Text("Sync is not configured.")
                Text("Quit Queue Focus and use the bundled qf-sync utility to preview and join your private server. Each device needs its own enrollment.")
                    .foregroundStyle(Color.readableGrey)
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
