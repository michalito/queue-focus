import os
import ServiceManagement

/// Opening Queue Focus at login, through the system's login items.
@MainActor
enum LoginItem {
    private static let log = Logger(subsystem: "org.queuefocus.QueueFocus", category: "login")

    static var isEnabled: Bool {
        SMAppService.mainApp.status == .enabled
    }

    /// Registered, but waiting for the user to allow it in System Settings.
    static var needsApproval: Bool {
        SMAppService.mainApp.status == .requiresApproval
    }

    static func set(_ enabled: Bool) {
        do {
            if enabled {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
        } catch {
            log.error("could not \(enabled ? "add" : "remove", privacy: .public) the login item: \(error.localizedDescription, privacy: .public)")
        }
    }

    static func openSystemSettings() {
        SMAppService.openSystemSettingsLoginItems()
    }
}
