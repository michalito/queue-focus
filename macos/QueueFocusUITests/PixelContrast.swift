import CoreGraphics

/// Text's contrast as its own pixels show it, by WCAG's formula, for a check
/// on XCTest's audit, which misjudges some wrapped lines. It answers only for
/// a picture that is plainly one colour of text on one background: the
/// background fills at least half of it, and every other pixel is the text's
/// colour or a blend of the two, as a glyph's edges are, with no blend common
/// enough to be a second colour of text. Otherwise it gives no answer, and the
/// audit's verdict stands. A few words in a grey between the text's and the
/// background's would pass for edges, so it is asked only about text the app
/// draws in one colour.
enum PixelContrast {
    static func measure(_ picture: CGImage) -> Double? {
        let (width, height) = (picture.width, picture.height)
        guard width > 0, height > 0,
              let space = CGColorSpace(name: CGColorSpace.sRGB),
              let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8,
                                      bytesPerRow: width * 4, space: space,
                                      bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue),
              let bytes = context.data?.assumingMemoryBound(to: UInt8.self)
        else { return nil }
        context.draw(picture, in: CGRect(x: 0, y: 0, width: width, height: height))
        var counts: [Colour: Int] = [:]
        for pixel in 0..<(width * height) {
            counts[Colour(bytes[pixel * 4], bytes[pixel * 4 + 1], bytes[pixel * 4 + 2]), default: 0] += 1
        }
        guard let background = counts.max(by: { $0.value < $1.value }),
              background.value * 2 >= width * height,
              // The glyphs' colour: the commonest of those clearly apart from
              // the background (each blend at their edges is rare).
              let text = counts.filter({ $0.key.contrast(with: background.key) >= 1.5 })
                  .max(by: { $0.value < $1.value })
        else { return nil }
        var strays = 0
        for (colour, count) in counts where colour != background.key && colour != text.key {
            guard let share = colour.share(of: text.key, over: background.key) else {
                strays += count
                continue
            }
            // A blend as common as a fifth of the text is more text.
            if share > 0.1, share < 0.9, count * 5 > text.value { return nil }
        }
        guard strays * 50 <= text.value else { return nil }
        return text.key.contrast(with: background.key)
    }

    struct Colour: Hashable {
        let red, green, blue: Double

        init(_ red: UInt8, _ green: UInt8, _ blue: UInt8) {
            (self.red, self.green, self.blue) = (Double(red), Double(green), Double(blue))
        }

        var luminance: Double {
            func linear(_ value: Double) -> Double {
                let value = value / 255
                return value <= 0.04045 ? value / 12.92 : pow((value + 0.055) / 1.055, 2.4)
            }
            return 0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue)
        }

        func contrast(with other: Colour) -> Double {
            let (a, b) = (luminance, other.luminance)
            return (max(a, b) + 0.05) / (min(a, b) + 0.05)
        }

        /// How much of `ink` this colour holds, if it is a blend of `ink`
        /// over `paper`, within a few levels in each channel.
        func share(of ink: Colour, over paper: Colour) -> Double? {
            let step = (ink.red - paper.red, ink.green - paper.green, ink.blue - paper.blue)
            let length = step.0 * step.0 + step.1 * step.1 + step.2 * step.2
            guard length > 0 else { return nil }
            let share = ((red - paper.red) * step.0 + (green - paper.green) * step.1 + (blue - paper.blue) * step.2) / length
            guard share >= -0.02, share <= 1.02 else { return nil }
            let off = max(abs(paper.red + share * step.0 - red), abs(paper.green + share * step.1 - green),
                          abs(paper.blue + share * step.2 - blue))
            return off <= 8 ? share : nil
        }
    }
}
