import SwiftUI

/// The Queue view: the current task in a banner, Side and Next in one
/// scrolling card, and Later on a shelf at the bottom.
struct QueueWindow: View {
    @Environment(QueueModel.self) private var model
    @State private var laterOpen = false

    var body: some View {
        let snapshot = model.snapshot
        VStack(spacing: 0) {
            NowBanner(current: snapshot.current)
                .padding(12)
            List {
                Section("Side") {
                    TaskRows(tasks: snapshot.side, empty: "Nothing on the side")
                }
                Section("Next") {
                    TaskRows(tasks: snapshot.next, empty: "Nothing queued")
                }
            }
            DisclosureGroup("Later (\(snapshot.later.count))", isExpanded: $laterOpen) {
                List {
                    TaskRows(tasks: snapshot.later, empty: "Nothing for later")
                }
                .frame(minHeight: 80, maxHeight: 220)
            }
            .padding(12)
        }
        .frame(minWidth: 320, minHeight: 420)
        .navigationTitle("Queue")
    }
}

/// The current task, or the invitation to pick one.
struct NowBanner: View {
    @Environment(QueueModel.self) private var model
    let current: QueueTask?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                SectionHeading("NOW")
                if let tag = current?.tag {
                    TagChip(tag: tag)
                }
                Spacer()
                if let current, let secs = model.elapsed(of: current) {
                    Text(longElapsed(secs: secs))
                        .font(.body.monospacedDigit())
                        .foregroundStyle(current.pausedAt == nil ? .secondary : .tertiary)
                }
            }
            if let current {
                Text(current.title)
                    .font(.title3.weight(.semibold))
                    .lineLimit(2)
                    .help(current.title)
            } else {
                Text("empty — promote one ↑")
                    .foregroundStyle(.secondary)
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(TagStyle.accent(current?.tag).opacity(current?.tag == nil ? 0.08 : 0.16),
                    in: RoundedRectangle(cornerRadius: 10))
    }
}

/// A bucket's tasks, one line each, or a word for an empty bucket.
struct TaskRows: View {
    let tasks: [QueueTask]
    let empty: String

    var body: some View {
        if tasks.isEmpty {
            Text(empty)
                .foregroundStyle(.tertiary)
        }
        ForEach(tasks, id: \.id) { task in
            HStack(spacing: 6) {
                if let tag = task.tag {
                    TagChip(tag: tag)
                }
                Text(task.title)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .help(task.title)
            }
        }
    }
}
