import SwiftUI

/// The Board view: the same tasks in four quadrants, for moving several at
/// once. Now and Next take the wide left three fifths, Side and Later the
/// narrow right two. Each is shaped for what it holds: the one task you are
/// doing is a panel, the queue behind it a card of rows, Side free-standing
/// cards, and Later a bare list.
struct BoardWindow: View {
    var body: some View {
        TaskWindow(page: .board) { focus in
            BoardQuadrants(focus: focus)
        }
        .frame(minWidth: 720, minHeight: 480)
        .navigationTitle("Board")
    }
}

private struct BoardQuadrants: View {
    @Environment(QueueModel.self) private var model
    @Environment(DragState.self) private var drag
    @Environment(\.adwAccent) private var accent
    var focus: FocusState<FocusTarget?>.Binding

    var body: some View {
        let snapshot = model.snapshot
        GeometryReader { geometry in
            let gap: CGFloat = 14
            // Five equal columns, spanned three and two, as GTK's grid has
            // them.
            let column = (geometry.size.width - 4 * gap) / 5
            let wide = column * 3 + gap * 2
            VStack(alignment: .leading, spacing: 6) {
                // The top row is as tall as the taller of its two.
                HStack(alignment: .top, spacing: gap) {
                    VStack(alignment: .leading, spacing: 0) {
                        // The Now heading takes a task as the panel does.
                        BoardHeading(bucket: .now, count: snapshot.current == nil ? 0 : 1, ink: accent.ink)
                            .padding(EdgeInsets(top: 8, leading: 4, bottom: 6, trailing: 4))
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .contentShape(Rectangle())
                            .onDrop(of: [.queueFocusTask], delegate: WholeDropDelegate(page: .board, mark: .ring(.now), drag: drag) { id in
                                model.promote(id: id)
                                return true
                            })
                            .accessibilityIdentifier("heading-now")
                        NowPanel(style: .card, focus: focus)
                            .frame(maxHeight: .infinity, alignment: .top)
                    }
                    .frame(width: wide)
                    .frame(maxHeight: .infinity, alignment: .top)
                    SideQuadrant(tasks: snapshot.side, focus: focus)
                        .frame(maxHeight: .infinity, alignment: .top)
                }
                .fixedSize(horizontal: false, vertical: true)
                HStack(alignment: .top, spacing: gap) {
                    Quadrant(bucket: .next, tasks: snapshot.next, focus: focus)
                        .frame(width: wide)
                    Quadrant(bucket: .later, tasks: snapshot.later, focus: focus)
                }
            }
        }
        .padding(EdgeInsets(top: 8, leading: 20, bottom: 14, trailing: 20))
    }
}

/// The Now quadrant's heading: the one in the accent, since it names what
/// you are doing.
private struct BoardHeading: View {
    let bucket: Bucket
    let count: Int
    let ink: Color

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(BucketName.of(bucket)).bucketHeaderStyle().foregroundStyle(ink)
            SectionCount(count: count)
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }
}

/// Next or Later: a bucket that fills its quadrant and scrolls; the room
/// under its last row takes a drop at the end. Next's rows sit in a card;
/// Later's are a bare list.
private struct Quadrant: View {
    let bucket: Bucket
    let tasks: [QueueTask]
    var focus: FocusState<FocusTarget?>.Binding

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // Aligned with the text of the rows under it: Next's card insets
            // them, and Later's column only just.
            BucketHeader(bucket: bucket, count: tasks.count,
                         insets: bucket == .next ? EdgeInsets(top: 10, leading: 12, bottom: 4, trailing: 12)
                                                 : EdgeInsets(top: 10, leading: 4, bottom: 4, trailing: 4))
            GeometryReader { viewport in
                ScrollViewReader { proxy in
                    ScrollView {
                        BucketList(bucket: bucket, tasks: tasks, style: RowStyle.of(.board, bucket),
                                   focus: focus, minHeight: viewport.size.height)
                    }
                    .onChange(of: focus.wrappedValue) { _, target in
                        if case .task(let id) = target, tasks.contains(where: { $0.id == id }) {
                            proxy.scrollTo(id)
                        }
                    }
                }
            }
            .modifier(QuadrantBody(card: bucket == .next))
        }
        .frame(maxHeight: .infinity)
        .bucketGroup(bucket)
    }
}

private struct QuadrantBody: ViewModifier {
    let card: Bool

    func body(content: Content) -> some View {
        if card {
            content.adwCard()
        } else {
            content.padding(.horizontal, 4)
        }
    }
}

/// Side grows with its cards to a point, then scrolls.
private struct SideQuadrant: View {
    let tasks: [QueueTask]
    var focus: FocusState<FocusTarget?>.Binding

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            BucketHeader(bucket: .side, count: tasks.count, insets: EdgeInsets(top: 8, leading: 4, bottom: 6, trailing: 4))
            ScrollView {
                // The 1 pt keeps the cards' rings off the scroller's edge,
                // and the extra at the top leaves room for a line above the
                // first card.
                BucketList(bucket: .side, tasks: tasks, style: .boardSide, focus: focus, minHeight: 70)
                    .padding(EdgeInsets(top: 5, leading: 1, bottom: 1, trailing: 1))
            }
            .frame(minHeight: 76, maxHeight: 202)
            .fixedSize(horizontal: false, vertical: true)
        }
        .bucketGroup(.side)
    }
}
