import SwiftUI

/// The Board view: the same tasks in four quadrants, for moving several at
/// once. Now and Next take the wide left three fifths, Side and Later the
/// narrow right two.
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
    var focus: FocusState<FocusTarget?>.Binding

    var body: some View {
        let snapshot = model.snapshot
        GeometryReader { geometry in
            let gap: CGFloat = 12
            let wide = (geometry.size.width - gap) * 3 / 5
            HStack(alignment: .top, spacing: gap) {
                VStack(alignment: .leading, spacing: gap) {
                    VStack(alignment: .leading, spacing: 6) {
                        // The Now heading takes a task as the panel does.
                        SectionHeading("NOW")
                            .padding(.horizontal, 8)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .contentShape(Rectangle())
                            .onDrop(of: [.queueFocusTask], delegate: WholeDropDelegate(mark: .ring(.now), drag: drag) { id in
                                model.promote(id: id)
                                return true
                            })
                            .accessibilityIdentifier("heading-now")
                        NowPanel(style: .card, focus: focus)
                    }
                    Quadrant(bucket: .next, tasks: snapshot.next, focus: focus)
                }
                .frame(width: wide)
                VStack(alignment: .leading, spacing: gap) {
                    SideQuadrant(tasks: snapshot.side, focus: focus)
                    Quadrant(bucket: .later, tasks: snapshot.later, focus: focus)
                }
            }
        }
        .padding([.horizontal, .bottom], 12)
    }
}

/// A bucket that fills its quadrant and scrolls; the room under its last
/// row takes a drop at the end.
private struct Quadrant: View {
    @Environment(WindowContext.self) private var context
    let bucket: Bucket
    let tasks: [QueueTask]
    var focus: FocusState<FocusTarget?>.Binding

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            BucketHeader(bucket: bucket, count: tasks.count)
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
        }
        .frame(maxHeight: .infinity)
    }
}

/// Side grows with its cards to a point, then scrolls.
private struct SideQuadrant: View {
    let tasks: [QueueTask]
    var focus: FocusState<FocusTarget?>.Binding

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            BucketHeader(bucket: .side, count: tasks.count)
            ScrollView {
                BucketList(bucket: .side, tasks: tasks, style: .boardSide, focus: focus, minHeight: 72)
            }
            .frame(minHeight: 72, maxHeight: 196)
            .fixedSize(horizontal: false, vertical: true)
        }
    }
}
