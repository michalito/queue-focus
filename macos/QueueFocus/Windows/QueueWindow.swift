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
    @Environment(WindowContext.self) private var context
    var focus: FocusState<FocusTarget?>.Binding

    var body: some View {
        let snapshot = model.snapshot
        VStack(spacing: 0) {
            NowPanel(style: .banner, focus: focus)
                .padding(.horizontal, 12)
            ScrollViewReader { proxy in
                ScrollView {
                    VStack(alignment: .leading, spacing: 6) {
                        BucketHeader(bucket: .side, count: snapshot.side.count)
                        BucketList(bucket: .side, tasks: snapshot.side, style: .queue, focus: focus)
                        Divider().padding(.vertical, 4)
                        BucketHeader(bucket: .next, count: snapshot.next.count)
                        BucketList(bucket: .next, tasks: snapshot.next, style: .queue, focus: focus)
                    }
                    .padding(12)
                }
                .onChange(of: focus.wrappedValue) { _, target in
                    if case .task(let id) = target { proxy.scrollTo(id) }
                }
            }
            LaterShelf(focus: focus)
        }
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
        VStack(alignment: .leading, spacing: 4) {
            Divider()
            Button {
                context.laterOpen.toggle()
            } label: {
                HStack(spacing: 6) {
                    Text("Later").font(.headline)
                    Text("\(later.count)").foregroundStyle(.secondary).monospacedDigit()
                    Spacer()
                    Image(systemName: context.laterOpen ? "chevron.down" : "chevron.right")
                        .foregroundStyle(.secondary)
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .dropRing(.laterShelf, in: drag, page: .queue, cornerRadius: 6)
            .appendDrop(.later, empty: later.isEmpty || !context.laterOpen, ring: .laterShelf, drag: drag, page: .queue, model: model)
            .accessibilityLabel("Later, \(later.count)")
            .accessibilityValue(context.laterOpen ? "open" : "closed")
            .accessibilityIdentifier("later-shelf")
            if context.laterOpen {
                ScrollView {
                    BucketList(bucket: .later, tasks: later, style: .later, focus: focus)
                }
                .frame(maxHeight: 180)
                .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding([.horizontal, .bottom], 12)
    }
}
