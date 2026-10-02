import AppKit
import Testing
@testable import QueueFocus

/// WCAG's contrast ratio between two sRGB colours, out of 255.
private func contrast(_ a: (Double, Double, Double), _ b: (Double, Double, Double)) -> Double {
    func luminance(_ c: (Double, Double, Double)) -> Double {
        func channel(_ v: Double) -> Double {
            let s = v / 255
            return s <= 0.039_28 ? s / 12.92 : pow((s + 0.055) / 1.055, 2.4)
        }
        return 0.2126 * channel(c.0) + 0.7152 * channel(c.1) + 0.0722 * channel(c.2)
    }
    let (high, low) = (max(luminance(a), luminance(b)), min(luminance(a), luminance(b)))
    return (high + 0.05) / (low + 0.05)
}

private let white = (255.0, 255.0, 255.0)

@MainActor
@Suite struct DisplayOptionsTests {
    @Test func theLaunchArgumentNamesTheSettings() {
        #expect(DisplayOptions(names: "increaseContrast, reduceTransparency")
            == DisplayOptions(increaseContrast: true, reduceTransparency: true))
        #expect(DisplayOptions(names: "reduceMotion,differentiateWithoutColor")
            == DisplayOptions(reduceMotion: true, differentiateWithoutColor: true))
        #expect(DisplayOptions(names: "") == DisplayOptions())
    }

    /// A chip's letter is small text: 4.5 to 1 at least, and more with
    /// Increase Contrast. GNOME's work blue, 3.8, would fail.
    @Test func whiteReadsOnEveryTagColour() {
        for tag in [TaskTag.work, .personal] {
            #expect(contrast(white, TagStyle.solidRGB(tag, increaseContrast: false)) >= 4.5, "\(tag)")
            #expect(contrast(white, TagStyle.solidRGB(tag, increaseContrast: true)) >= 6, "\(tag), more contrast")
        }
        #expect(contrast(white, (0x35, 0x84, 0xE4)) < 4.5, "the reason the chip is not GNOME's accent")
    }

    @Test func withoutColourTheMenuBarNamesTheTag() {
        let title = StatusTitle(tag: .work, title: "ship v0.1", fullTitle: "ship v0.1", timer: "12m", paused: false)
        let font = NSFont.menuBarFont(ofSize: 0)
        #expect(!StatusItemController.render(title, font: font, options: DisplayOptions()).string.contains("W"))
        #expect(StatusItemController.render(title, font: font, options: DisplayOptions(differentiateWithoutColor: true))
            .string.hasPrefix("● W ship v0.1"))
    }

    @Test func withoutTransparencyTheFlashCardIsSolid() throws {
        let plan = FlashPlan.make(flashEvent(.wash), screen: CGSize(width: 800, height: 600), menuBar: 30, opaqueCard: true)
        #expect(plan.card.opaque)
        let image = try #require(FlashCardView.render(plan.card, scale: 1))
        // Inside the card's left padding, clear of the rounded corners.
        let context = try #require(CGContext(data: nil, width: image.width, height: image.height, bitsPerComponent: 8,
                                             bytesPerRow: 0, space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                             bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        let bytes = try #require(context.data).assumingMemoryBound(to: UInt8.self)
        let alpha = { (x: Int, y: Int) in Int(bytes[y * context.bytesPerRow + x * 4 + 3]) }
        #expect(alpha(10, image.height / 2) == 255)
        let seeThrough = try #require(FlashCardView.render(FlashPlan.make(flashEvent(.wash), screen: CGSize(width: 800, height: 600),
                                                                          menuBar: 30).card, scale: 1))
        context.clear(CGRect(x: 0, y: 0, width: context.width, height: context.height))
        context.draw(seeThrough, in: CGRect(x: 0, y: 0, width: seeThrough.width, height: seeThrough.height))
        #expect(alpha(10, seeThrough.height / 2) < 255, "and otherwise it is see-through")
    }
}
