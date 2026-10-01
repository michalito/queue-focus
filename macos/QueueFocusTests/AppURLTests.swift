import Foundation
import Testing
@testable import QueueFocus

private func link(_ string: String) -> AppURL? {
    URL(string: string).flatMap(AppURL.init)
}

@Suite struct AppURLTests {
    @Test func addTakesTheTextAndWhetherItIsCurrent() {
        #expect(link("queuefocus://add?text=Write%20the%20report") == .add(text: "Write the report", asCurrent: false))
        #expect(link("queuefocus://add?text=ship&now=1") == .add(text: "ship", asCurrent: true))
        #expect(link("QueueFocus://ADD?text=ship&now=true") == .add(text: "ship", asCurrent: true))
        #expect(link("queuefocus://add?text=ship&now=0") == .add(text: "ship", asCurrent: false))
        #expect(link("queuefocus://add?text=first&text=second") == .add(text: "first", asCurrent: false))
        #expect(link("queuefocus://add?text=%20%20") == nil, "nothing to add")
        #expect(link("queuefocus://add") == nil)
    }

    @Test func showOpensAView() {
        #expect(link("queuefocus://show") == .show(.queue))
        #expect(link("queuefocus://show?view=board") == .show(.board))
        #expect(link("queuefocus://show?view=add") == .show(.quickAdd))
        #expect(link("queuefocus://show?view=settings") == nil)
    }

    @Test func nothingElseIsALink() {
        #expect(link("queuefocus://complete") == nil, "no completing behind the user's back")
        #expect(link("queuefocus://") == nil)
        #expect(link("https://add?text=ship") == nil)
    }
}

@MainActor
@Suite struct LinksTests {
    @Test func aLinkBeforeTheQueueIsOpenWaitsForIt() throws {
        var ignored: [URL] = []
        var followed: [AppURL] = []
        let links = Links { ignored.append($0) }
        links.receive([try #require(URL(string: "queuefocus://add?text=a")), try #require(URL(string: "queuefocus://nonsense"))])
        #expect(followed.isEmpty)
        #expect(ignored.map(\.absoluteString) == ["queuefocus://nonsense"])
        links.open { followed.append($0) }
        #expect(followed == [.add(text: "a", asCurrent: false)])
        links.receive([try #require(URL(string: "queuefocus://show?view=board"))])
        #expect(followed == [.add(text: "a", asCurrent: false), .show(.board)], "after that, at once")
    }
}
