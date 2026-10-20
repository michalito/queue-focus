import CoreGraphics
import CoreText
import XCTest

/// The measure that checks the audit, on pictures drawn here: it answers for
/// one colour of text on one background, and only then.
final class PixelContrastTests: XCTestCase {
    private typealias Grey = CGFloat

    /// A grey in sRGB, the picture's space (a generic grey would be converted).
    private func srgb(_ grey: Grey) -> CGColor {
        CGColor(srgbRed: grey, green: grey, blue: grey, alpha: 1)
    }

    /// Words in greys on a grey, as Core Text draws them (edges blended), and
    /// a frame of another grey round the edge if asked for.
    private func picture(background: Grey, words: [(String, Grey)], frame: Grey? = nil) throws -> CGImage {
        let (width, height) = (480, 48)
        let context = try XCTUnwrap(CGContext(data: nil, width: width, height: height, bitsPerComponent: 8,
                                              bytesPerRow: width * 4, space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                              bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue))
        context.setFillColor(srgb(background))
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        let font = CTFontCreateWithName("Helvetica" as CFString, 26, nil)
        var x: CGFloat = 8
        for (text, grey) in words {
            let line = CTLineCreateWithAttributedString(NSAttributedString(string: text, attributes: [
                .font: font, .foregroundColor: srgb(grey),
            ]))
            context.textPosition = CGPoint(x: x, y: 16)
            CTLineDraw(line, context)
            x += CTLineGetTypographicBounds(line, nil, nil, nil) + 8
        }
        if let frame {
            context.setStrokeColor(srgb(frame))
            context.setLineWidth(8)
            context.stroke(CGRect(x: 0, y: 0, width: width, height: height))
        }
        return try XCTUnwrap(context.makeImage())
    }

    func testOneGreyOnOneBackgroundIsMeasured() throws {
        // #616161 on #f7f7f7, the readable grey on a light form's row.
        let measured = try XCTUnwrap(PixelContrast.measure(picture(background: 247 / 255, words: [
            ("Picks one of six styles at random", 97 / 255),
        ])))
        XCTAssertEqual(measured, 5.78, accuracy: 0.05)
    }

    func testAFaintGreyMeasuresFaint() throws {
        let measured = try XCTUnwrap(PixelContrast.measure(picture(background: 1, words: [
            ("Picks one of six styles at random", 0.6),
        ])))
        XCTAssertLessThan(measured, 4.5)
    }

    func testTwoGreysOfTextGetNoAnswer() throws {
        // Dark words beside faint ones: the dark would pass, the faint not.
        // More of the faint: the dark is no blend of the faint and the paper.
        XCTAssertNil(PixelContrast.measure(try picture(background: 1, words: [
            ("dark words", 0x33 / 255), ("faint words, more of them", 0x99 / 255),
        ])))
        // More of the dark: the faint is a blend too common to be an edge.
        XCTAssertNil(PixelContrast.measure(try picture(background: 1, words: [
            ("dark words, more of them", 0x33 / 255), ("faint words", 0x99 / 255),
        ])))
    }

    func testAFrameOfAnotherColourGetsNoAnswer() throws {
        // White on #eeeeee fails; a dark frame in the picture must not stand
        // in for the text.
        XCTAssertNil(PixelContrast.measure(try picture(background: 0xEE / 255, words: [
            ("white words here", 1),
        ], frame: 0x22 / 255)))
    }
}
