import AppKit
import ServiceManagement
import Testing
@testable import QueueFocus

private func task(_ title: String, tag: TaskTag? = nil, paused: UInt64? = nil) -> QueueTask {
    QueueTask(id: 1, title: title, bucket: .now, tag: tag, createdAt: 0, startedAt: 0, pausedAt: paused)
}

/// The task's title in the centre of the menu bar: where the label goes, and
/// where it gives way to the status item.
@Suite struct CentreTitleTests {
    /// An ultrawide display, its menu bar 30 points tall.
    private let wide = CentreTitle.Screen(frame: CGRect(x: 0, y: 0, width: 3360, height: 1418),
                                          visibleFrame: CGRect(x: 0, y: 0, width: 3360, height: 1388))

    @Test func theLabelFillsTheMenuBarAroundTheScreensMiddle() throws {
        let frame = try #require(CentreTitle.frame(on: wide, words: 200, dotMinX: 2000))
        #expect(frame == CGRect(x: 1572, y: 1388, width: 216, height: 30))
        #expect(frame.midX == wide.frame.midX)
    }

    /// Wider or narrower words leave its middle where it was, so the popover
    /// under it does not move.
    @Test func itsMiddleStaysPutWhateverItsWords() throws {
        for words: CGFloat in [40, 41.3, 97.5, 200, 333.3] {
            let frame = try #require(CentreTitle.frame(on: wide, words: words, dotMinX: nil))
            #expect(frame.midX == 1680, "\(words) points of words: \(frame)")
            #expect(frame.width >= words + 2 * CentreTitle.padding)
            #expect(frame.minX == frame.minX.rounded() && frame.width == frame.width.rounded(), "on whole points")
        }
    }

    @Test func aScreenOffToOneSideCentresOnItself() throws {
        let second = CentreTitle.Screen(frame: CGRect(x: -1920, y: 200, width: 1920, height: 1080),
                                        visibleFrame: CGRect(x: -1920, y: 200, width: 1920, height: 1055))
        let frame = try #require(CentreTitle.frame(on: second, words: 100, dotMinX: nil))
        #expect(frame.midX == -960)
        #expect(frame.minY == 1255 && frame.height == 25)
    }

    /// The label keeps clear of the app's own item, the one thing in the
    /// menu bar it can see.
    @Test func itGivesWayWhereItWouldReachTheItem() {
        // 216 wide: its right edge is at 1788.
        #expect(CentreTitle.frame(on: wide, words: 200, dotMinX: 1788 + CentreTitle.gap) != nil, "just clear")
        #expect(CentreTitle.frame(on: wide, words: 200, dotMinX: 1788 + CentreTitle.gap - 1) == nil, "a point short")
        #expect(CentreTitle.frame(on: wide, words: 200, dotMinX: 1500) == nil, "the item is left of the middle")
    }

    @Test func aNotchTakesTheCentre() {
        var notched = wide
        notched.hasNotch = true
        #expect(CentreTitle.frame(on: notched, words: 200, dotMinX: nil) == nil)
    }

    /// A menu bar that hides itself leaves the screen whole.
    @Test func aScreenWithNoMenuBarInViewHasNoPlaceForIt() {
        let bare = CentreTitle.Screen(frame: wide.frame, visibleFrame: wide.frame)
        #expect(CentreTitle.frame(on: bare, words: 200, dotMinX: nil) == nil)
    }

    @Test func wordsWiderThanTheScreenDoNotFit() {
        let small = CentreTitle.Screen(frame: CGRect(x: 0, y: 0, width: 300, height: 200),
                                       visibleFrame: CGRect(x: 0, y: 0, width: 300, height: 176))
        #expect(CentreTitle.frame(on: small, words: 400, dotMinX: nil) == nil)
    }

    /// The label is no button: nothing gives its words a colour but the
    /// attributes, where the status item's take the menu bar's.
    @Test @MainActor func theLabelsWordsAllHaveAColour() {
        let font = NSFont.menuBarFont(ofSize: 0)
        for paused: UInt64? in [nil, 60] {
            let title = StatusTitle.make(current: task("ship", tag: .work, paused: paused), now: 600, showTimer: true,
                                         maxWidth: 200, measure: { Double($0.count) })
            let text = StatusItemController.render(title, font: font, options: DisplayOptions(differentiateWithoutColor: true),
                                                   color: .labelColor)
            text.enumerateAttribute(.foregroundColor, in: NSRange(location: 0, length: text.length)) { colour, range, _ in
                #expect(colour != nil, "\((text.string as NSString).substring(with: range).debugDescription), paused \(paused != nil)")
            }
            let words = (text.string as NSString).range(of: "ship")
            let colour = text.attribute(.foregroundColor, at: words.location, effectiveRange: nil) as? NSColor
            #expect(colour == (paused == nil ? .labelColor : StatusItemController.pausedTitleColor))
        }
    }

    /// The menu bar's appearance is a vibrant one, its colours made for the
    /// material under them; the label is on a bare panel. It takes the menu
    /// bar's light or dark and the plain colours of that, which read: the
    /// vibrant dark's grey for no tag is near black, on a dark menu bar.
    @Test @MainActor func theLabelTakesTheMenuBarsLightOrDarkWithoutItsVibrancy() throws {
        func luminance(_ colour: NSColor, in name: NSAppearance.Name, over ground: CGFloat) throws -> CGFloat {
            var resolved: NSColor?
            try #require(NSAppearance(named: name)).performAsCurrentDrawingAppearance {
                resolved = colour.usingColorSpace(.sRGB)
            }
            let c = try #require(resolved)
            let channel = { (value: CGFloat) -> CGFloat in
                let mixed = value * c.alphaComponent + ground * (1 - c.alphaComponent)
                return mixed <= 0.03928 ? mixed / 12.92 : pow((mixed + 0.055) / 1.055, 2.4)
            }
            return 0.2126 * channel(c.redComponent) + 0.7152 * channel(c.greenComponent) + 0.0722 * channel(c.blueComponent)
        }
        for (menuBar, plain, ground) in [(NSAppearance.Name.vibrantDark, NSAppearance.Name.darkAqua, CGFloat(0)),
                                         (.vibrantLight, .aqua, 1), (.darkAqua, .darkAqua, 0), (.aqua, .aqua, 1)] {
            let name = CentreTitle.appearance(matching: try #require(NSAppearance(named: menuBar)))
            #expect(name == plain, "\(menuBar.rawValue)")
            // The words, a paused task's, and the dot of no tag, against the
            // menu bar at its darkest or lightest.
            let bar = ground == 0 ? CGFloat(0) : 1
            for (colour, least) in [(NSColor.labelColor, CGFloat(7)), (StatusItemController.pausedTitleColor, 4.5),
                                    (TagStyle.dot(nil), 1.3)] {
                let ink = try luminance(colour, in: name, over: ground)
                let contrast = (max(ink, bar) + 0.05) / (min(ink, bar) + 0.05)
                #expect(contrast >= least, "\(colour) on a \(plain.rawValue) menu bar: \(contrast)")
            }
        }
    }

    /// While the label carries the title the item keeps the dot, and the
    /// tag's letter where colour should not be the only difference.
    @Test @MainActor func theItemKeepsTheDot() {
        let font = NSFont.menuBarFont(ofSize: 0)
        let title = StatusTitle.make(current: task("ship", tag: .work), now: 600, showTimer: true, maxWidth: 200,
                                     measure: { Double($0.count) })
        #expect(StatusItemController.renderDot(title, font: font, options: DisplayOptions()).string == "●")
        #expect(StatusItemController.renderDot(title, font: font, options: DisplayOptions(differentiateWithoutColor: true)).string == "● W")
        let empty = StatusTitle.make(current: nil, now: 0, showTimer: true, maxWidth: 200, measure: { Double($0.count) })
        #expect(StatusItemController.renderDot(empty, font: font, options: DisplayOptions(differentiateWithoutColor: true)).string == "●")
    }
}

/// Stands in for the system's login items, which a test leaves alone.
@MainActor
private final class NoLoginItems: LoginItemService {
    let status = SMAppService.Status.notRegistered
    func register() throws {}
    func unregister() throws {}
}

/// Where the label goes, which a test can move.
@MainActor
private final class Place {
    var screen: CentreTitle.Screen?

    init(_ screen: CentreTitle.Screen?) {
        self.screen = screen
    }
}

/// The setting puts the label up and takes it down, on defaults of the
/// test's own and a screen of its own, far from the real one, where nothing
/// is seen. One at a time: each looks for the label among the app's windows.
@MainActor
@Suite(.serialized) struct CentreTitleSettingTests {
    private static let screen = CentreTitle.Screen(frame: CGRect(x: 50_000, y: 50_000, width: 2000, height: 1000),
                                                   visibleFrame: CGRect(x: 50_000, y: 50_000, width: 2000, height: 975))

    /// The real screen has a place for the label, and it can be seen there.
    private static var theMenuBarsCentreIsFree: Bool {
        guard QuickAddTests.screenIsUnlocked, let screen = NSScreen.screens.first else { return false }
        return CentreTitle.frame(on: CentreTitle.Screen(screen), words: 150, dotMinX: nil) != nil
    }

    private func eventually(_ condition: () -> Bool) async throws -> Bool {
        for _ in 0..<40 {
            if condition() { return true }
            try await Task.sleep(for: .milliseconds(50))
        }
        return condition()
    }

    private var label: CentreTitlePanel? {
        NSApp.windows.compactMap { $0 as? CentreTitlePanel }.first { $0.isVisible }
    }

    @Test func theSettingPutsTheLabelUpAndTakesItDown() async throws {
        let suite = "qf-centre-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(suite, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer {
            defaults.removePersistentDomain(forName: suite)
            try? FileManager.default.removeItem(at: dir)
        }
        let model = QueueModel(engine: try QueueEngine(dir: dir.path))
        model.add("ship the release #w", asCurrent: true)
        let controller = StatusItemController(model: model, loginItem: LoginItem(service: NoLoginItems()), windows: AppWindows(),
                                              display: Display(defaults: defaults), defaults: defaults,
                                              screen: { Self.screen }, quit: {})
        model.didChange = { [weak controller] in controller?.update() }
        #expect(label == nil, "off unless chosen")
        #expect(controller.itemWords.contains("ship the release"))

        // Settings writes the defaults, and nothing else tells the menu bar.
        defaults.set(true, forKey: Preferences.menuBarTitleCentredKey)
        #expect(try await eventually { label != nil }, "the setting puts the label up")
        let panel = try #require(label)
        #expect(panel.label.title.string.hasPrefix("● ship the release"))
        #expect(panel.label.spokenTitle.hasPrefix("Queue Focus: ship the release"))
        #expect(panel.frame.midX == Self.screen.frame.midX)
        #expect(panel.frame.minY == Self.screen.visibleFrame.maxY && panel.frame.height == 25, "in the menu bar")
        #expect(panel.frame.width >= panel.label.title.size().width + 2 * CentreTitle.padding)

        // New words, wider: the same middle.
        model.add("write the notes for the release after it #p", asCurrent: true)
        #expect(panel.label.title.string.hasPrefix("● write the notes"))
        #expect(panel.frame.midX == Self.screen.frame.midX)
        #expect(panel.frame.width >= panel.label.title.size().width + 2 * CentreTitle.padding)

        // Off the real screens, the label is in no one's view.
        #expect(controller.itemWords.contains("write the notes"), "a label out of view leaves the item its title")

        defaults.set(false, forKey: Preferences.menuBarTitleCentredKey)
        #expect(try await eventually { label == nil }, "and takes it down")
    }

    /// The item gives its title up only to a label in view, and takes it
    /// back from one that is not: here, one moved off the screen, as a
    /// full-screen space or Mission Control takes it out of view.
    @Test(.enabled("this Mac's menu bar has no free centre in view") { await theMenuBarsCentreIsFree })
    func theItemKeepsTheDotOnlyWhileTheLabelIsInView() async throws {
        let suite = "qf-centre-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(suite, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer {
            defaults.removePersistentDomain(forName: suite)
            try? FileManager.default.removeItem(at: dir)
        }
        defaults.set(true, forKey: Preferences.menuBarTitleCentredKey)
        let model = QueueModel(engine: try QueueEngine(dir: dir.path))
        model.add("hand over", asCurrent: true)
        let place = Place(NSScreen.screens.first.map { CentreTitle.Screen($0) })
        let controller = StatusItemController(model: model, loginItem: LoginItem(service: NoLoginItems()), windows: AppWindows(),
                                              display: Display(defaults: defaults), defaults: defaults,
                                              screen: { place.screen }, quit: {})
        try #require(try await eventually { label != nil }, "no room for the label beside this menu bar's icons")
        #expect(try await eventually { controller.itemWords == "●" }, "in view, the label has the title: \(controller.itemWords)")

        place.screen = Self.screen
        controller.update()
        #expect(label != nil, "still up, where no one sees it")
        #expect(try await eventually { controller.itemWords.contains("hand over") },
                "out of view, the item has the title: \(controller.itemWords)")

        place.screen = NSScreen.screens.first.map { CentreTitle.Screen($0) }
        controller.update()
        #expect(try await eventually { controller.itemWords == "●" }, "and back in view, the dot")

        defaults.set(false, forKey: Preferences.menuBarTitleCentredKey)
        #expect(try await eventually { label == nil })
        #expect(controller.itemWords.contains("hand over"))
    }

    /// Where the label has no place, there is none, and the item keeps the
    /// title: here, a screen whose middle a notch takes.
    @Test func withNoPlaceForItThereIsNoLabel() async throws {
        let suite = "qf-centre-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(suite, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer {
            defaults.removePersistentDomain(forName: suite)
            try? FileManager.default.removeItem(at: dir)
        }
        defaults.set(true, forKey: Preferences.menuBarTitleCentredKey)
        var notched = Self.screen
        notched.hasNotch = true
        let model = QueueModel(engine: try QueueEngine(dir: dir.path))
        let controller = StatusItemController(model: model, loginItem: LoginItem(service: NoLoginItems()), windows: AppWindows(),
                                              display: Display(defaults: defaults), defaults: defaults,
                                              screen: { notched }, quit: {})
        model.add("ship", asCurrent: true)
        controller.update()
        try await Task.sleep(for: .milliseconds(200))
        #expect(label == nil)
    }
}
