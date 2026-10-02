import os
import ServiceManagement

/// Opening Queue Focus at login, through the system's login items. One
/// instance for the app, so the gear menu and Settings always show the same
/// thing; it reads the system again after every change and whenever the app
/// comes forward, since System Settings can change it too.
@MainActor
@Observable
final class LoginItem {
    private(set) var isEnabled = false
    /// Registered, but waiting for the user to allow it in System Settings.
    private(set) var needsApproval = false
    private let log = Logger(subsystem: "org.queuefocus.QueueFocus", category: "login")

    init() {
        refresh()
    }

    func refresh() {
        let status = SMAppService.mainApp.status
        isEnabled = status == .enabled
        needsApproval = status == .requiresApproval
    }

    func set(_ enabled: Bool) {
        do {
            if enabled {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
        } catch {
            log.error("could not \(enabled ? "add" : "remove", privacy: .public) the login item: \(error.localizedDescription, privacy: .public)")
        }
        refresh()
    }

    func openSystemSettings() {
        SMAppService.openSystemSettingsLoginItems()
    }
}
