import Foundation

/// A data directory set up for sync, as `qf-sync setup` leaves it, whose
/// server is never there: the queue stays offline. Removed with the fixture.
struct OfflineSyncDirectory {
    let url: URL

    init() throws {
        url = FileManager.default.temporaryDirectory
            .appendingPathComponent("qf-sync-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        let device = UUID().uuidString.lowercased()
        let envelope: [String: Any] = [
            "version": 1, "device": device, "server_time": 0, "wall_time": 0,
            "pending": [], "handles": [:], "next_handle": 1, "recovery": [], "suspension": NSNull(),
            "confirmed": [
                "version": 1, "dataset": UUID().uuidString, "generation": UUID().uuidString,
                "revision": 1, "history": String(repeating: "0", count: 64), "initialized": true,
                "tasks": [], "focus": ["version": UUID().uuidString, "session": NSNull()], "undo": NSNull(),
            ],
        ]
        let config: [String: Any] = [
            "version": 1, "url": "http://127.0.0.1:1", "device": device,
            "token": "localtest", "allow_loopback_http": true,
        ]
        for (name, value) in [("sync-state.json", envelope), ("sync-config.json", config)] {
            let file = url.appendingPathComponent(name)
            try JSONSerialization.data(withJSONObject: value).write(to: file)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: file.path)
        }
    }

    func remove() {
        try? FileManager.default.removeItem(at: url)
    }
}
