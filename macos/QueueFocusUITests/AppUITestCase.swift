import XCTest

/// The app launched on a data directory of the test's own, and what the
/// tests need to drive it.
@MainActor
class AppUITestCase: XCTestCase {
    /// The global shortcuts, moved for the tests to keys nothing else uses
    /// (Control-Option-Shift with J, L, K and U), so a test never presses the
    /// real ones, which another copy of the app, or another app, may hold.
    /// They go in the launch arguments, so the user's settings stay as they
    /// are. (Function keys typed by XCTest never match a hot key.)
    static let testShortcuts: [(name: String, key: XCUIKeyboardKey, carbonKeyCode: Int)] = [
        ("toggleQueue", XCUIKeyboardKey(rawValue: "j"), 38),
        ("quickAdd", XCUIKeyboardKey(rawValue: "l"), 37),
        ("showBoard", XCUIKeyboardKey(rawValue: "k"), 40),
        ("completeCurrent", XCUIKeyboardKey(rawValue: "u"), 32),
    ]
    static let testShortcutModifiers: XCUIElement.KeyModifierFlags = [.control, .option, .shift]
    private static let carbonModifiers = 4096 + 2048 + 512

    /// Press the global shortcut for `name`, as moved for the tests.
    func pressShortcut(_ name: String) {
        guard let shortcut = Self.testShortcuts.first(where: { $0.name == name }) else {
            return XCTFail("no shortcut \(name)")
        }
        app.typeKey(shortcut.key, modifierFlags: Self.testShortcutModifiers)
    }

    var app: XCUIApplication!
    var dataHome: URL!

    /// Launch the app on a fresh data directory, removed after the test,
    /// holding `tasks` if given: `(title, bucket)` pairs, and `settings`.
    func launch(with tasks: [(String, String)] = [], settings: [String: Any] = [:], arguments: [String] = [],
                restoringState: Bool = false) throws {
        continueAfterFailure = false
        let dataHome = FileManager.default.temporaryDirectory
            .appendingPathComponent("qf-ui-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dataHome, withIntermediateDirectories: true)
        let dir = dataHome.appendingPathComponent("queue-focus")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        if !settings.isEmpty {
            try JSONSerialization.data(withJSONObject: settings).write(to: dir.appendingPathComponent("settings.json"))
        }
        if !tasks.isEmpty {
            let file: [String: Any] = [
                "next_id": tasks.count + 1,
                "tasks": tasks.enumerated().map { index, task in
                    ["id": index + 1, "title": task.0, "bucket": task.1, "created_at": 0]
                },
            ]
            try JSONSerialization.data(withJSONObject: file).write(to: dir.appendingPathComponent("tasks.json"))
        }
        let app = XCUIApplication()
        app.launchEnvironment["XDG_DATA_HOME"] = dataHome.path
        app.launchArguments += ["-allowSecondInstance", "YES"] + arguments
        // Asking for notifications would put the system's prompt on screen.
        app.launchArguments += ["-notifications", "off"]
        for shortcut in Self.testShortcuts {
            // A string in the arguments' plist syntax, holding the JSON
            // KeyboardShortcuts stores.
            let json = #"{\"carbonKeyCode\":\#(shortcut.carbonKeyCode),\"carbonModifiers\":\#(Self.carbonModifiers)}"#
            app.launchArguments += ["-KeyboardShortcuts_\(shortcut.name)", "\"\(json)\""]
        }
        if restoringState {
            // As for someone who keeps windows when quitting an app.
            app.launchArguments += ["-ApplePersistenceIgnoreState", "NO", "-NSQuitAlwaysKeepsWindows", "YES"]
        }
        app.launch()
        addTeardownBlock { @MainActor in
            app.terminate()
            try? FileManager.default.removeItem(at: dataHome)
        }
        self.app = app
        self.dataHome = dataHome
        XCTAssertTrue(statusItem.waitForExistence(timeout: 10), "the status item appears")
        XCTAssertEqual(app.windows.count, 0, "a menu bar app opens no window at launch: \(app.windows.debugDescription)")
    }

    /// The saved queue, as the engine wrote it.
    func storedTitles() throws -> [String] {
        let data = try Data(contentsOf: dataHome.appendingPathComponent("queue-focus/tasks.json"))
        let file = try JSONSerialization.jsonObject(with: data) as? [String: Any]
        let tasks = file?["tasks"] as? [[String: Any]] ?? []
        return tasks.compactMap { $0["title"] as? String }
    }

    var statusItem: XCUIElement {
        app.statusItems["queue-focus-status-item"]
    }

    var statusTitle: String {
        statusItem.title.isEmpty ? (statusItem.value as? String ?? "") : statusItem.title
    }

    func openPopover() -> XCUIElement {
        statusItem.click()
        let field = app.textFields["add-field"]
        XCTAssertTrue(field.waitForExistence(timeout: 5), "the popover opens with its add field")
        return field
    }

    func waitForTitle(containing text: String, file: StaticString = #filePath, line: UInt = #line) {
        let predicate = NSPredicate { _, _ in MainActor.assumeIsolated { self.statusTitle.contains(text) } }
        let found = XCTNSPredicateExpectation(predicate: predicate, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [found], timeout: 5), .completed,
                       "status title \(statusTitle.debugDescription) never contained \(text)", file: file, line: line)
    }

    /// The saved tasks as (id, title, bucket), in the file's order.
    func storedTasks() throws -> [(id: UInt64, title: String, bucket: String)] {
        let data = try Data(contentsOf: dataHome.appendingPathComponent("queue-focus/tasks.json"))
        let file = try JSONSerialization.jsonObject(with: data) as? [String: Any]
        let tasks = file?["tasks"] as? [[String: Any]] ?? []
        return tasks.compactMap { task in
            guard let id = task["id"] as? Int, let title = task["title"] as? String,
                  let bucket = task["bucket"] as? String else { return nil }
            return (UInt64(id), title, bucket)
        }
    }

    /// The saved titles in one bucket, in order.
    func stored(_ bucket: String) throws -> [String] {
        try storedTasks().filter { $0.bucket == bucket }.map(\.title)
    }

    /// Wait until the saved queue satisfies `check`.
    func waitForStore(_ description: String, file: StaticString = #filePath, line: UInt = #line,
                      _ check: @escaping @MainActor () throws -> Bool) {
        let predicate = NSPredicate { _, _ in MainActor.assumeIsolated { (try? check()) ?? false } }
        let met = XCTNSPredicateExpectation(predicate: predicate, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [met], timeout: 5), .completed, description, file: file, line: line)
    }

    /// Click a control once it is there: a popover's contents can arrive a
    /// moment after the popover does.
    func click(_ element: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertTrue(element.waitForExistence(timeout: 5), "\(element) is there to click", file: file, line: line)
        element.click()
    }

    /// Open a window from the popover.
    func openWindow(_ button: String, title: String) -> XCUIElement {
        _ = openPopover()
        click(app.buttons[button])
        let window = app.windows[title]
        XCTAssertTrue(window.waitForExistence(timeout: 5), "\(title) opens")
        return window
    }
}
