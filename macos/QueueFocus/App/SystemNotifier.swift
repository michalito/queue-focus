import Foundation
import UserNotifications

/// Notifications through Notification Center. Made at launch, so the Done
/// note's Undo button is known before an answer to it can arrive, even one
/// that launches the app.
@MainActor
final class SystemNotifier: NSObject, Notifier, UNUserNotificationCenterDelegate {
    private nonisolated static let doneCategory = "done"
    private nonisolated static let undoAction = "undo"
    private nonisolated static let taskKey = "task"

    private let center = UNUserNotificationCenter.current()
    /// Told when a Done note's Undo is pressed, with the task's id.
    var onUndo: @MainActor (UInt64) -> Void = { _ in }

    override init() {
        super.init()
        center.delegate = self
        // Undo runs in the background: the app stays where it was.
        let undo = UNNotificationAction(identifier: Self.undoAction, title: "Undo", options: [])
        center.setNotificationCategories([
            UNNotificationCategory(identifier: Self.doneCategory, actions: [undo], intentIdentifiers: []),
        ])
    }

    func permission() async -> NotePermission {
        switch await center.notificationSettings().authorizationStatus {
        case .authorized, .provisional: .allowed
        case .notDetermined: .notAsked
        default: .denied
        }
    }

    func ask() {
        center.requestAuthorization(options: [.alert]) { _, _ in }
    }

    func post(_ note: Note) {
        let content = UNMutableNotificationContent()
        content.title = note.title
        if let body = note.body {
            content.body = body
        }
        if let task = note.undo {
            content.categoryIdentifier = Self.doneCategory
            content.userInfo = [Self.taskKey: String(task)]
        }
        center.add(UNNotificationRequest(identifier: note.id, content: content, trigger: nil))
    }

    func withdraw(_ id: String) {
        center.removeDeliveredNotifications(withIdentifiers: [id])
        center.removePendingNotificationRequests(withIdentifiers: [id])
    }

    /// Shown even while the popover or a window of ours is in front.
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter,
                                            willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        [.banner, .list]
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter,
                                            didReceive response: UNNotificationResponse) async {
        guard response.actionIdentifier == Self.undoAction,
              let task = (response.notification.request.content.userInfo[Self.taskKey] as? String).flatMap(UInt64.init)
        else { return }
        await MainActor.run { onUndo(task) }
    }
}

/// No notifications: as when they are turned off, so everything goes to the
/// popover. For `-notifications off`, which the UI tests pass, since asking
/// would put the system's prompt on the screen.
@MainActor
final class SilentNotifier: Notifier {
    func permission() async -> NotePermission { .denied }
    func ask() {}
    func post(_ note: Note) {}
    func withdraw(_ id: String) {}
}
