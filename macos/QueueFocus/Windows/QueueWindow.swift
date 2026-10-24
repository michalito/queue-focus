import SwiftUI

/// The Queue view, in three bands: the current task in a banner, Side and
/// Next in one scrolling card, and Later on a shelf pinned to the bottom.
struct QueueWindow: View {
    var body: some View {
        TaskWindow(page: .queue) { focus in
            QueueBands(focus: focus)
        }
        .frame(minWidth: 320, minHeight: 420)
        .navigationTitle("Queue")
    }
}

private struct QueueBands: View {
    @Environment(QueueModel.self) private var model
    var focus: FocusState<FocusTarget?>.Binding

    var body: some View {
        let snapshot = model.snapshot
        VStack(spacing: 0) {
            NowPanel(style: .banner, focus: focus)
            ScrollViewReader { proxy in
                ScrollView {
                    // One flat card holds Side and Next, split by a hairline.
                    VStack(alignment: .leading, spacing: 0) {
                        QueueSection(bucket: .side, tasks: snapshot.side, divided: false, focus: focus)
                        QueueSection(bucket: .next, tasks: snapshot.next, divided: true, focus: focus)
                    }
                    .adwCard()
                    .padding(.horizontal, 12)
                    .padding(.vertical, 14)
                }
                .onChange(of: focus.wrappedValue) { _, target in
                    if case .task(let id) = target { proxy.scrollTo(id) }
                }
            }
            LaterShelf(focus: focus)
        }
    }
}

/// A bucket in the Queue's card: its heading, then its rows or `empty`.
private struct QueueSection: View {
    let bucket: Bucket
    let tasks: [QueueTask]
    let divided: Bool
    var focus: FocusState<FocusTarget?>.Binding

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if divided {
                Hairline().padding(.top, 6)
            }
            BucketHeader(bucket: bucket, count: tasks.count, insets: EdgeInsets(top: 10, leading: 12, bottom: 4, trailing: 12))
            BucketList(bucket: bucket, tasks: tasks, style: .queue, focus: focus)
        }
        .bucketGroup(bucket)
    }
}

/// Later, pinned below the scrolling card so it never pushes the queue
/// around. It starts closed; `l` or its heading opens it. A drop on the
/// heading lands at the end of Later, open or not.
private struct LaterShelf: View {
    @Environment(QueueModel.self) private var model
    @Environment(DragState.self) private var drag
    @Environment(WindowContext.self) private var context
    var focus: FocusState<FocusTarget?>.Binding

    var body: some View {
        let later = model.snapshot.later
        VStack(alignment: .leading, spacing: 0) {
            Hairline()
            Button {
                context.laterOpen.toggle()
            } label: {
                HStack(spacing: 8) {
                    Text("Later").bucketHeaderStyle().opacity(0.75)
                    SectionCount(count: later.count)
                    Spacer()
                    Image(systemName: context.laterOpen ? "chevron.down" : "chevron.right")
                        .font(.system(size: 11, weight: .bold))
                        .opacity(0.6)
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 10)
                .contentShape(Rectangle())
            }
            .buttonStyle(AdwButtonStyle(padding: EdgeInsets(), radius: 0))
            .dropRing(.laterShelf, in: drag, page: .queue, cornerRadius: 0)
            .appendDrop(.later, empty: later.isEmpty || !context.laterOpen, ring: .laterShelf, drag: drag, page: .queue, model: model)
            .accessibilityLabel("Later, \(later.count)")
            .accessibilityValue(context.laterOpen ? "open" : "closed")
            .accessibilityIdentifier("later-shelf")
            if context.laterOpen {
                ScrollView {
                    BucketList(bucket: .later, tasks: later, style: .later, focus: focus)
                        .padding(EdgeInsets(top: 0, leading: 16, bottom: 12, trailing: 16))
                }
                .frame(maxHeight: 180)
                .fixedSize(horizontal: false, vertical: true)
            }
        }
        .background(Adw.card)
    }
}
