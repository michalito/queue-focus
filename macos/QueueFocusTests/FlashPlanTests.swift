import CoreGraphics
import Testing
@testable import QueueFocus

private let screen = CGSize(width: 1920, height: 1080)
private let menuBar = 30.0
private let whole = CGRect(origin: .zero, size: screen)

func flashEvent(_ style: FlashStyle, intensity: Intensity = .normal, palette: Palette = .blue,
                title: String = "ship v0.1", timer: String = "23m") -> FlashEvent {
    FlashEvent(style: style, intensity: intensity, palette: palette, title: title, timer: timer)
}

private func plan(_ style: FlashStyle, intensity: Intensity = .normal, palette: Palette = .blue,
                  title: String = "ship v0.1", timer: String = "23m", on size: CGSize = screen,
                  menuBar bar: Double = menuBar) -> FlashPlan {
    FlashPlan.make(flashEvent(style, intensity: intensity, palette: palette, title: title, timer: timer),
                   screen: size, menuBar: bar)
}

private func strips(_ layer: FlashLayer) -> [(frame: CGRect, start: RGBA, end: RGBA)] {
    layer.parts.compactMap { part in
        if case .gradient(let start, let end, _) = part.shape { (part.frame, start, end) } else { nil }
    }
}

/// Ported from the GNOME extension's flash tests (extension/test/flash.test.mjs).
/// Its last case, a style or palette this version does not know, cannot
/// happen here: the engine hands over enums, not names.
@Suite struct FlashPlanTests {
    @Test func everyStyleCoversTheScreenAndPutsTheCardOnTop() {
        for style in FlashPreview.allStyles {
            let plan = plan(style)
            #expect(plan.screen == screen)
            #expect(plan.layers.count >= 2, "\(style) draws something besides the card")
            let card = plan.layers.last
            #expect(card?.frame == whole)
            #expect(card?.parts == [FlashPart(frame: whole, shape: .card)])
            #expect(card?.envelope == .card)
            #expect(card?.motion == Motion(property: .translationY, from: -4, to: 0, fraction: 0.2), "it slides down into place")
            #expect(plan.layers.dropLast().allSatisfy { $0.parts.allSatisfy { $0.shape != .card } })
            #expect(plan.layers.allSatisfy { whole.contains($0.frame) }, "\(style) stays on the screen")
            #expect(plan.card == FlashCard(title: "ship v0.1", timer: "23m", color: RGBA(hex: 0x62a0ea), scale: 1))
        }
    }

    @Test func aTaskWhoseClockHasNotStartedDropsTheTimerLine() {
        let card = plan(.wash, timer: "").card
        #expect(card.timer == nil)
        #expect(FlashController.label(card) == "Queue Focus flash: NOW, ship v0.1")
    }

    @Test func anUntrustedTitleIsBoundedBeforeItIsLaidOut() {
        let title = plan(.wash, title: "  first\n\t" + String(repeating: "x", count: 10000)).card.title
        #expect(title.unicodeScalars.count <= 256, "a hard cap")
        #expect(title.hasPrefix("first x"), "whitespace folds into one paragraph")
        #expect(title.hasSuffix("…"), "the cut shows")
    }

    @Test func boundingDoesNotScanThroughAnUnboundedWhitespaceTail() {
        #expect(safeTitle("first" + String(repeating: " ", count: 10000) + "last") == "first…")
    }

    @Test func aTitleThatFitsIsOnlyTidied() {
        #expect(safeTitle("  ship \t\n v0.1  ") == "ship v0.1")
        #expect(safeTitle("\u{FEFF}a\u{00A0}b") == "a b", "JavaScript's white space")
        #expect(safeTitle("") == "")
        let longest = String(repeating: "x", count: 255)
        #expect(safeTitle(longest) == longest)
        #expect(safeTitle(longest + "y") == longest + "…")
        // Counted as JavaScript counts: by code point.
        #expect(safeTitle(String(repeating: "🦀", count: 300)).unicodeScalars.count == 256)
    }

    @Test func thePersonalPaletteColoursTheCardNotTheTitle() {
        let plan = plan(.wash, palette: .orange)
        #expect(plan.card.color == RGBA(hex: 0xffa348), "NOW and the clock take the accent")
        #expect(plan.layers[0].parts[0].shape == .fill(RGBA(230, 97, 0, 0.26)), "the wash goes orange")
    }

    @Test func theGlowRunsTheWholeOfEachSideCornersIncluded() {
        for (style, inset) in [(FlashStyle.edges, 8.0), (.edgesSoft, 0)] {
            let found = strips(plan(style).layers[0]).map(\.frame)
            #expect(found.count == 4, "one strip per side")
            for strip in found {
                #expect(strip.width > 0 && strip.height > 0)
                #expect(strip.minX >= inset && strip.minY >= inset, "clear of the ring")
                #expect(strip.maxX <= screen.width - inset && strip.maxY <= screen.height - inset)
            }
            // Every corner is covered twice, and the middle by none.
            let covering = { (x: Double, y: Double) in found.filter { $0.contains(CGPoint(x: x, y: y)) }.count }
            let far = (x: Double(screen.width) - inset - 1, y: Double(screen.height) - inset - 1)
            for (x, y) in [(inset, inset), (far.x, inset), (inset, far.y), (far.x, far.y)] {
                #expect(covering(x, y) == 2, "\(style) corner \(x),\(y)")
            }
            #expect(covering(960, 540) == 0)
        }
    }

    @Test func theGlowFadesToAClearAccentNeverToBlack() {
        let accent = RGBA(53, 132, 228)
        for strip in strips(plan(.edgesSoft).layers[0]) {
            #expect([strip.start, strip.end].contains(accent.alpha(0)))
            #expect([strip.start, strip.end].contains(accent.alpha(0.8 * 0.85)))
        }
    }

    @Test func theEdgesRingSitsUnderAGlowThatStartsInsideIt() {
        let layer = plan(.edges).layers[0]
        #expect(layer.envelope == .edges)
        #expect(layer.parts[0] == FlashPart(frame: whole, shape: .border(width: 8, RGBA(53, 132, 228, 0.85))))
        #expect(strips(layer).allSatisfy { $0.start.alpha == 0.7 * 0.85 || $0.end.alpha == 0.7 * 0.85 })
        #expect(plan(.edgesSoft).layers[0].envelope == .breath, "the soft edges breathe")
    }

    @Test func aScreenTooSmallForTheGlowStillGetsOne() {
        let found = strips(plan(.edgesSoft, intensity: .strong, on: CGSize(width: 120, height: 90)).layers[0])
        #expect(found.count == 4)
        #expect(found.allSatisfy { $0.frame.width > 0 && $0.frame.height > 0 })
    }

    @Test func theBeamRunsFromTheMenuBarDownToTheMiddleOfTheScreen() {
        let plan = plan(.topbarBeam)
        #expect(plan.layers[0].frame == CGRect(x: 0, y: 0, width: 1920, height: 30), "the bar covers the menu bar exactly")
        #expect(plan.layers[0].envelope == .bar)
        let beam = plan.layers[1]
        #expect(beam.frame == CGRect(x: 959, y: 30, width: 3, height: 510))
        #expect(beam.parts == [FlashPart(frame: CGRect(x: 0, y: 0, width: 3, height: 510), shape: .fill(RGBA(53, 132, 228)))])
        #expect(beam.pivot == CGPoint(x: 0.5, y: 0), "so it grows downwards")
        #expect(beam.motion == Motion(property: .scaleY, from: 0, to: 1, fraction: 0.3))
        #expect(beam.envelope == .beam)
    }

    @Test func thePlainTopBarStyleDrawsNoBeam() {
        #expect(plan(.topbar).layers.count == 2, "the bar and the card, nothing else")
        #expect(plan(.topbarBeam, menuBar: 540).layers.count == 2, "a bar down to the middle leaves no room for one")
    }

    @Test func intensityChangesHowLoudAFlashIs() {
        for (intensity, wash, scale) in [(Intensity.subtle, 0.14, 0.92), (.normal, 0.26, 1), (.strong, 0.42, 1.08)] {
            let plan = plan(.wash, intensity: intensity)
            #expect(plan.layers[0].parts[0].shape == .fill(RGBA(53, 132, 228, wash)))
            #expect(plan.card.scale == scale)
        }
    }

    @Test func theEnvelopesAreFlashJsOwn() {
        #expect(Envelope.wash.values == [0, 1, 1, 0] && Envelope.wash.keyTimes == [0, 0.25, 0.70, 1])
        #expect(Envelope.wash2.values == [0, 1, 0.45, 1, 0])
        #expect(Envelope.edges.values == [0, 1, 0.5, 1, 0], "a deeper dip than wash2's")
        #expect(Envelope.breath.values == [0, 1, 0] && Envelope.breath.duration == 1.8)
        #expect(Envelope.beam.values == [1, 1, 0], "the beam starts lit and grows")
        for envelope in [Envelope.wash, .wash2, .edges, .breath, .bar, .beam, .card] {
            #expect(envelope.keyTimes == envelope.keyTimes.sorted() && envelope.keyTimes.last == 1)
            #expect(envelope.end == 0 && envelope.peak == 1)
        }
        #expect(plan(.wash).duration == 1.7, "as long as its longest layer, the card")
        #expect(plan(.edgesSoft).duration == 1.8)
    }
}
