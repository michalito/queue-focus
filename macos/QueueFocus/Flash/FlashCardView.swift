import SwiftUI

/// The card every flash shows: NOW, the task, and its clock, as the GNOME
/// stylesheet's `qf-flash-card` draws it.
struct FlashCardView: View {
    let card: FlashCard

    var body: some View {
        HuggingWidth(min: 460, max: 860) {
            VStack(spacing: 8) {
                Text("NOW")
                    .font(.system(size: 13, weight: .heavy))
                    .tracking(1)
                    .foregroundStyle(color(card.color))
                Text(card.title)
                    .font(.system(size: 34, weight: .heavy))
                    .foregroundStyle(.white)
                    .multilineTextAlignment(.center)
                    .lineLimit(3)
                    .truncationMode(.tail)
                if let timer = card.timer {
                    Text(timer)
                        .font(.system(size: 24, weight: .semibold, design: .monospaced))
                        .monospacedDigit()
                        .foregroundStyle(color(card.color))
                }
            }
        }
        .padding(EdgeInsets(top: 26, leading: 44, bottom: 24, trailing: 44))
        .background(Color(red: 6 / 255, green: 6 / 255, blue: 10 / 255, opacity: 0.85),
                    in: RoundedRectangle(cornerRadius: 18))
        .environment(\.colorScheme, .dark)
    }

    private func color(_ color: RGBA) -> Color {
        Color(red: color.red / 255, green: color.green / 255, blue: color.blue / 255, opacity: color.alpha)
    }

    /// The card drawn once, `scale` pixels to the point: the flash animates
    /// a picture of it rather than laying text out on every frame.
    @MainActor
    static func render(_ card: FlashCard, scale: CGFloat) -> CGImage? {
        let renderer = ImageRenderer(content: FlashCardView(card: card))
        renderer.scale = scale
        return renderer.cgImage
    }
}

/// As wide as its content wants, between `min` and `max`: the card hugs a
/// short title and wraps a long one, as St sizes the GNOME card.
private struct HuggingWidth: Layout {
    let min: CGFloat
    let max: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard let content = subviews.first else { return .zero }
        let size = content.sizeThatFits(ProposedViewSize(width: max, height: nil))
        return CGSize(width: Swift.min(Swift.max(size.width, min), max), height: size.height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        subviews.first?.place(at: CGPoint(x: bounds.midX, y: bounds.minY), anchor: .top,
                              proposal: ProposedViewSize(width: bounds.width, height: nil))
    }
}
