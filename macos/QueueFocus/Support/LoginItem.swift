import os
import ServiceManagement

/// What `LoginItem` asks of the system, so tests can stand in for it without
/// registering anything.
@MainActor
protocol LoginItemService {
    var status: SMAppService.Status { get }
    func register() throws
    func unregister() throws
}

extension SMAppService: LoginItemService {}

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
    private let service: LoginItemService
    private let log = Logger(subsystem: "org.queuefocus.QueueFocus", category: "login")

    init(service: LoginItemService = SMAppService.mainApp) {
        self.service = service
        refresh()
    }

    func refresh() {
        let status = service.status
        isEnabled = status == .enabled
        needsApproval = status == .requiresApproval
    }

    func set(_ enabled: Bool) {
        do {
            if enabled {
                try service.register()
            } else {
                try service.unregister()
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
