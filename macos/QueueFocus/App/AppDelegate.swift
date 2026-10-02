import AppKit
import os

/// Opens the engine, puts the task in the menu bar and keeps the clock.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private(set) var model: QueueModel?
    /// Launch at login, as the gear menu and Settings both show it.
    let loginItem = LoginItem()
    /// The one drag in progress, shared by the windows.
    let drag = DragState()
    /// The Queue and Board windows, for the global shortcuts and links.
    let windows = AppWindows()
    private var statusItem: StatusItemController?
    /// The floating add field, which ⌘N and the global shortcut open.
    private(set) var quickAdd: QuickAddController?
    private var flash: FlashController?
    private var notices: Notices?
    private lazy var links = Links { [log] url in
        log.notice("ignored a link it does not know: \(url.absoluteString, privacy: .public)")
    }
    private var ticker: Timer?
    private var theme: Theme?
    private let log = Logger(subsystem: "org.queuefocus.QueueFocus", category: "app")

    func applicationDidFinishLaunching(_ notification: Notification) {
        // Hosted unit tests run inside this app; they make engines of their
        // own, and must never tick the user's.
        guard !Launch.isRunningUnitTests else { return }
        guard !Launch.anotherInstanceIsRunning else {
            log.notice("another Queue Focus is already running; quitting")
            NSApp.terminate(nil)
            return
        }

        let dir = defaultDataDir()
        let engine: QueueEngine
        do {
            engine = try QueueEngine(dir: dir)
        } catch {
            // The task file is not replaced with an empty queue: say why and stop.
            Alerts.show(
                "Queue Focus cannot read its tasks",
                "\(QueueModel.describe(error))\n\nRepair or move the file in \(dir), then open Queue Focus again.",
                style: .critical
            )
            NSApp.terminate(nil)
            return
        }

        let model = QueueModel(engine: engine)
        self.model = model
        IntentHost.model = model
        let statusItem = StatusItemController(model: model, loginItem: loginItem, windows: windows, defaults: .standard, quit: {
            NSApp.terminate(nil)
        })
        self.statusItem = statusItem
        quickAdd = QuickAddController(model: model)
        let flash = FlashController()
        self.flash = flash
        model.presentFlash = { [weak flash] event in flash?.show(event) }
        NotificationCenter.default.addObserver(self, selector: #selector(screensDidChange),
                                               name: NSApplication.didChangeScreenParametersNotification, object: nil)
        let notices = Notices(model: model, notifier: makeNotifier(), popoverIsShown: { statusItem.isPopoverShown },
                              showPopover: { statusItem.showPopover() })
        self.notices = notices
        // The record a Done note from before could undo went with the app.
        notices.withdrawDone()
        model.didReport = { problems in
            notices.report(problems)
        }
        Hotkey.install { [weak self] in self?.perform($0) }
        model.didChange = { [weak self] in self?.modelDidChange() }
        modelDidChange()
        startTicking(model)

        if let warning = engine.openWarning() {
            Alerts.show("Queue Focus could not read its settings", warning, style: .warning)
        }
        FlashPreview.schedule(FlashPreview.events(.standard), on: flash)
        links.open { [weak self] in self?.follow($0) }
    }

    /// Notification Center's, unless `-notifications off` asks for none.
    private func makeNotifier() -> Notifier {
        guard UserDefaults.standard.string(forKey: "notifications") != "off" else { return SilentNotifier() }
        let notifier = SystemNotifier()
        notifier.onUndo = { [weak self] id in
            self?.notices?.undo(id: id)
        }
        return notifier
    }

    private func perform(_ hotkey: Hotkey) {
        switch hotkey {
        case .toggleQueue:
            windows.toggle(WindowID.queue)
        case .showBoard:
            windows.show(WindowID.board)
        case .quickAdd:
            quickAdd?.show()
        case .completeCurrent:
            notices?.completeCurrent()
        }
    }

    func application(_ application: NSApplication, open urls: [URL]) {
        links.receive(urls)
    }

    private func follow(_ link: AppURL) {
        guard let model, let notices else { return }
        LinkFollower(model: model, show: { [weak self] view in
            switch view {
            case .queue: self?.windows.show(WindowID.queue)
            case .board: self?.windows.show(WindowID.board)
            case .quickAdd: self?.quickAdd?.show()
            }
        }, failed: { title, reason in
            notices.failed(title, reason)
        }).follow(link)
    }

    /// A flash drawn for a screen that has since changed goes at once.
    @objc private func screensDidChange(_ notification: Notification) {
        flash?.clear()
    }

    /// System Settings can change the login item while the app is away.
    func applicationDidBecomeActive(_ notification: Notification) {
        loginItem.refresh()
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        ticker?.invalidate()
        notices?.withdrawDone()
        // Settings are written a moment after they change; this is the one
        // time that wait cannot be afforded.
        for problem in model?.flush() ?? [] {
            log.error("\(problem, privacy: .public)")
        }
        return .terminateNow
    }

    /// One tick a second, in every run loop mode so an open menu does not
    /// hold it up. The tick writes changed settings and keeps the reminder.
    private func startTicking(_ model: QueueModel) {
        let timer = Timer(timeInterval: 1, repeats: true) { _ in
            MainActor.assumeIsolated { model.tick() }
        }
        timer.tolerance = 0.1
        RunLoop.main.add(timer, forMode: .common)
        ticker = timer
    }

    private func modelDidChange() {
        statusItem?.update()
        notices?.modelDidChange()
        guard let model, model.settings.theme != theme else { return }
        theme = model.settings.theme
        NSApp.appearance = switch model.settings.theme {
        case .system: nil
        case .light: NSAppearance(named: .aqua)
        case .dark: NSAppearance(named: .darkAqua)
        }
    }
}

/// What the app needs to know about how it was started.
enum Launch {
    /// Inside a hosted unit test run.
    static var isRunningUnitTests: Bool {
        ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] != nil
    }

    /// Two copies would be two engines rewriting one task file. LaunchServices
    /// stops a second launch of the same app bundle, but not of a second copy
    /// of it, such as a development build beside the installed one. UI tests
    /// use their own data and pass `-allowSecondInstance YES`.
    static var anotherInstanceIsRunning: Bool {
        guard !UserDefaults.standard.bool(forKey: "allowSecondInstance"),
              let id = Bundle.main.bundleIdentifier
        else { return false }
        return NSRunningApplication.runningApplications(withBundleIdentifier: id)
            .contains { $0 != .current }
    }
}

@MainActor
enum Alerts {
    /// An alert in front of everything else; a menu bar app is never in front
    /// on its own.
    static func show(_ message: String, _ information: String, style: NSAlert.Style) {
        let alert = NSAlert()
        alert.messageText = message
        alert.informativeText = information
        alert.alertStyle = style
        NSApp.activate()
        alert.runModal()
    }
}
