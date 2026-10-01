import Foundation

/// A `queuefocus:` link, for the shell and other apps:
///
///     open "queuefocus://add?text=Write%20the%20report"   # &now=1: as current
///     open "queuefocus://show?view=board"                  # queue, board, add
///
/// Completing a task is left out: any web page can open a link, after one
/// prompt, and a completion should not happen behind the user's back. The
/// global shortcut and Shortcuts do that.
enum AppURL: Equatable {
    case add(text: String, asCurrent: Bool)
    case show(View)

    enum View: Equatable {
        case queue
        case board
        case quickAdd
    }

    static let scheme = "queuefocus"

    init?(_ url: URL) {
        guard url.scheme?.lowercased() == Self.scheme,
              let components = URLComponents(url: url, resolvingAgainstBaseURL: false)
        else { return nil }
        var query: [String: String] = [:]
        for item in components.queryItems ?? [] where query[item.name] == nil {
            query[item.name] = item.value ?? ""
        }
        switch components.host?.lowercased() {
        case "add":
            guard let text = query["text"], !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
            self = .add(text: text, asCurrent: ["1", "true", "yes"].contains(query["now"]?.lowercased() ?? ""))
        case "show":
            switch query["view"]?.lowercased() ?? "queue" {
            case "queue": self = .show(.queue)
            case "board": self = .show(.board)
            case "add": self = .show(.quickAdd)
            default: return nil
            }
        default:
            return nil
        }
    }
}

/// The links that arrive. One that starts the app arrives before the queue is
/// open, and is held until it is; after that each is followed as it comes.
@MainActor
final class Links {
    private var follow: (@MainActor (AppURL) -> Void)?
    private var held: [AppURL] = []
    /// Told of a link that means nothing.
    private let ignore: @MainActor (URL) -> Void

    init(ignore: @escaping @MainActor (URL) -> Void) {
        self.ignore = ignore
    }

    func receive(_ urls: [URL]) {
        for url in urls {
            guard let link = AppURL(url) else {
                ignore(url)
                continue
            }
            if let follow {
                follow(link)
            } else {
                held.append(link)
            }
        }
    }

    /// The queue is open: follow what was held, then whatever comes.
    func open(_ follow: @escaping @MainActor (AppURL) -> Void) {
        self.follow = follow
        let held = held
        self.held = []
        held.forEach(follow)
    }
}

