import ServiceManagement
import Testing
@testable import QueueFocus

/// Stands in for the system's login items.
@MainActor
private final class FakeService: LoginItemService {
    var status: SMAppService.Status = .notRegistered
    var refuses = false
    private(set) var calls: [String] = []

    struct Refused: Error {}

    func register() throws {
        calls.append("register")
        if refuses { throw Refused() }
        status = .enabled
    }

    func unregister() throws {
        calls.append("unregister")
        if refuses { throw Refused() }
        status = .notRegistered
    }
}

@MainActor
@Suite struct LoginItemTests {
    @Test func turningItOnAndOffFollowsTheSystem() {
        let service = FakeService()
        let item = LoginItem(service: service)
        #expect(!item.isEnabled)
        item.set(true)
        #expect(item.isEnabled)
        item.set(false)
        #expect(!item.isEnabled)
        #expect(service.calls == ["register", "unregister"])
    }

    /// A refusal leaves the toggle showing what the system says, not what
    /// was asked for.
    @Test func aRefusalShowsTheSystemsAnswer() {
        let service = FakeService()
        service.refuses = true
        let item = LoginItem(service: service)
        item.set(true)
        #expect(!item.isEnabled)
    }

    /// System Settings can change it while the app is away; a refresh reads it.
    @Test func aChangeElsewhereIsReadOnRefresh() {
        let service = FakeService()
        let item = LoginItem(service: service)
        service.status = .requiresApproval
        #expect(!item.needsApproval)
        item.refresh()
        #expect(item.needsApproval)
        #expect(!item.isEnabled)
    }
}
