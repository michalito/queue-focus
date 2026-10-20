// The golden data files of crates/qf-core/tests/fixtures, read through the
// Swift bindings: the files the GNOME app writes are the Mac's too.
import Foundation
import QfCore
import Testing

/// crates/qf-core/tests/fixtures, from this file's place in the repository.
private let fixtures = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .appendingPathComponent("../../../../crates/qf-core/tests/fixtures")
    .standardizedFileURL

@Test func theGoldenFilesReadThroughTheBindings() throws {
    // Copies: reading a file tightens its permissions.
    let dir = FileManager.default.temporaryDirectory.appendingPathComponent("qf-golden-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: dir) }
    for name in ["tasks.json", "settings.json"] {
        try FileManager.default.copyItem(at: fixtures.appendingPathComponent(name), to: dir.appendingPathComponent(name))
    }
    let engine = try QueueEngine(dir: dir.path)
    #expect(engine.openWarning() == nil)

    let snapshot = engine.snapshot()
    let current = try #require(snapshot.current)
    #expect(current.id == 3 && current.title == "Ship v0.5" && current.tag == .work)
    #expect(current.startedAt == 1_759_310_000 && current.pausedAt == 1_759_311_000)
    #expect(snapshot.next.map(\.title) == ["Write the release notes", "Café ☕ with Ana"])
    #expect(snapshot.next.map(\.tag) == [nil, .personal])
    #expect(snapshot.later.map(\.title) == ["Renew the domain"])
    #expect(snapshot.side.map(\.title) == ["Reply to the review"])

    let settings = engine.settings()
    #expect(settings.intervalMin == 25 && !settings.vary && settings.intensity == .strong && settings.color == .orange)
    #expect(!settings.quietPaused && settings.quietHours && !settings.showTimer)
    #expect(settings.quietFrom == TimeOfDay(hour: 8, minute: 30) && settings.quietTo == TimeOfDay(hour: 17, minute: 45))
    #expect(settings.theme == .dark && settings.defaultBucket == .later)
}
