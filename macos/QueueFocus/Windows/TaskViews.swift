import AppKit
import SwiftUI

/// What can hold the keyboard in a task window.
enum FocusTarget: Hashable {
    /// A task's row, or the current task's panel.
    case task(UInt64)
    case add
    case rename(UInt64)
}

/// What one task window keeps between redraws.
@MainActor
@Observable
final class WindowContext {
    let page: Page
    /// The Queue's Later shelf; the Board shows Later always.
    var laterOpen = false
    /// The task being renamed in place, and the title typed so far. Kept
    /// here, not in the field, so a change from elsewhere does not lose it.
    var renaming: UInt64?
    var renameDraft = ""
    var showShortcuts = false

    init(page: Page) {
        self.page = page
    }

    func beginRename(_ task: QueueTask) {
        renameDraft = task.title
        renaming = task.id
        // A Later task is renamed where it can be seen.
        if page == .queue, task.bucket == .later {
            laterOpen = true
        }
    }
}

/// The style of a task row, by page and bucket.
enum RowStyle {
    case queue, later, boardSide, boardNext, boardLater

    static func of(_ page: Page, _ bucket: Bucket) -> RowStyle {
        switch (page, bucket) {
        case (.queue, .later): .later
        case (.queue, _): .queue
        case (.board, .side): .boardSide
        case (.board, .later): .boardLater
        case (.board, _): .boardNext
        }
    }

    /// How many lines a title may take before it is cut.
    var lines: Int {
        switch self {
        case .queue, .later, .boardLater: 1
        case .boardSide: 2
        case .boardNext: 3
        }
    }

    /// The Queue's rows carry their buttons; the Board's narrower quadrants
    /// keep only the menu.
    var inlineButtons: Bool {
        self == .queue || self == .later
    }

    var gap: CGFloat {
        self == .boardSide ? 8 : 4
    }
}

// MARK: Titles

/// A title cut to `lines`, with the whole of it in a tooltip only when it
/// was cut: a title that fits shows no tooltip.
struct TruncatedTitle: View {
    let text: String
    let lines: Int
    var font: Font = .body
    @State private var shownHeight: CGFloat = 0
    @State private var fullHeight: CGFloat = 0

    var body: some View {
        Text(text)
            .font(font)
            .lineLimit(lines)
            .truncationMode(.tail)
            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { shownHeight = $0 }
            .background(alignment: .topLeading) {
                // The same text at the same width without a limit: taller
                // means the one shown was cut.
                Text(text)
                    .font(font)
                    .fixedSize(horizontal: false, vertical: true)
                    .hidden()
                    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { fullHeight = $0 }
            }
            .help(fullHeight > shownHeight + 0.5 ? text : "")
    }
}

// MARK: Menus

/// The task menu, in a row's context menu and behind its menu button.
struct TaskMenuItems: View {
    @Environment(QueueModel.self) private var model
    @Environment(WindowContext.self) private var context
    let task: QueueTask
    let isCurrent: Bool

    var body: some View {
        if !isCurrent {
            Button("Make Current") { model.promote(id: task.id) }
        }
        Picker("Tag", selection: Binding(
            get: { task.tag },
            set: { model.setTag(id: task.id, $0) }
        )) {
            Text("No Tag").tag(TaskTag?.none)
            Text("Work").tag(TaskTag?.some(.work))
            Text("Personal").tag(TaskTag?.some(.personal))
        }
        Button("Done") { model.complete(id: task.id) }
        Divider()
        Menu("Move To") {
            ForEach([Bucket.side, .next, .later].filter { $0 != task.bucket }, id: \.self) { bucket in
                Button(BucketName.of(bucket)) { model.move(id: task.id, to: bucket) }
            }
        }
        Button("Rename") { context.beginRename(task) }
        Divider()
        Button("Delete", role: .destructive) { model.remove(id: task.id) }
    }
}

enum BucketName {
    static func of(_ bucket: Bucket) -> String {
        switch bucket {
        case .now: "Now"
        case .next: "Next"
        case .later: "Later"
        case .side: "Side"
        }
    }
}

/// The `…` button that opens the task menu.
struct TaskMenuButton: View {
    let task: QueueTask
    let isCurrent: Bool

    var body: some View {
        Menu {
            TaskMenuItems(task: task, isCurrent: isCurrent)
        } label: {
            Image(systemName: "ellipsis")
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .accessibilityLabel("Task menu")
        .accessibilityIdentifier("menu-\(task.id)")
    }
}

// MARK: Rename

/// A title being renamed in place. Return saves, Escape cancels, and so does
/// moving the keyboard elsewhere in the window; switching to another app
/// keeps the edit. A blank title is not saved.
struct RenameField: View {
    @Environment(QueueModel.self) private var model
    @Environment(WindowContext.self) private var context
    @Environment(\.controlActiveState) private var activeState
    let task: QueueTask
    var focus: FocusState<FocusTarget?>.Binding
    var font: Font = .body

    var body: some View {
        @Bindable var context = context
        TextField("Title", text: $context.renameDraft)
            .font(font)
            .textFieldStyle(.roundedBorder)
            .focused(focus, equals: .rename(task.id))
            .onSubmit(save)
            .onKeyPress(.escape) {
                finish()
                return .handled
            }
            .onChange(of: context.renameDraft) { _, draft in
                let limit = Int(maxTitleChars())
                if draft.count > limit { context.renameDraft = String(draft.prefix(limit)) }
            }
            .onChange(of: focus.wrappedValue) { _, now in
                // Focus passes through nothing on its way in; only a move to
                // something else in the window ends the rename.
                if let now, now != .rename(task.id), activeState == .key, context.renaming == task.id {
                    context.renaming = nil
                }
            }
            .onAppear {
                // Once laid out, the field takes the keyboard, and a fresh
                // rename selects the old title to type over it.
                DispatchQueue.main.async {
                    focus.wrappedValue = .rename(task.id)
                    DispatchQueue.main.async {
                        NSApp.sendAction(#selector(NSText.selectAll(_:)), to: nil, from: nil)
                    }
                }
            }
            .accessibilityIdentifier("rename-field")
    }

    private func save() {
        model.rename(id: task.id, to: context.renameDraft)
        finish()
    }

    /// The keyboard goes back to the task once its row can take it again.
    private func finish() {
        context.renaming = nil
        let id = task.id
        DispatchQueue.main.async {
            focus.wrappedValue = .task(id)
        }
    }
}

// MARK: Rows

/// One task in a list: its tag, its title, and what can be done with it.
struct TaskRow: View {
    @Environment(QueueModel.self) private var model
    @Environment(DragState.self) private var drag
    @Environment(WindowContext.self) private var context
    let task: QueueTask
    let style: RowStyle
    var focus: FocusState<FocusTarget?>.Binding

    var body: some View {
        let focused = focus.wrappedValue == .task(task.id)
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            if let tag = task.tag {
                TagChip(tag: tag)
            }
            Group {
                if context.renaming == task.id {
                    RenameField(task: task, focus: focus)
                } else {
                    TruncatedTitle(text: task.title, lines: style.lines)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if style.inlineButtons {
                Button {
                    model.promote(id: task.id)
                } label: {
                    Image(systemName: "arrow.up")
                }
                .help("Make current")
                .accessibilityLabel("Make current")
                .accessibilityIdentifier("promote-\(task.id)")
                if style == .later {
                    Button("→ next") { model.move(id: task.id, to: .next) }
                        .accessibilityIdentifier("to-next-\(task.id)")
                }
            }
            TaskMenuButton(task: task, isCurrent: false)
        }
        .buttonStyle(.borderless)
        .padding(.horizontal, 8)
        .padding(.vertical, style == .boardSide ? 8 : 5)
        .background(background, in: RoundedRectangle(cornerRadius: 7))
        .overlay {
            if focused {
                RoundedRectangle(cornerRadius: 7)
                    .strokeBorder(Color(nsColor: .keyboardFocusIndicatorColor), lineWidth: 2)
            }
        }
        .opacity(drag.fades(task.id) ? 0.35 : (style == .boardLater ? 0.7 : 1))
        .contentShape(Rectangle())
        // While renamed, the field inside takes the keyboard instead.
        .focusable(context.renaming != task.id)
        .focused(focus, equals: .task(task.id))
        .focusEffectDisabled()
        .onTapGesture(count: 2) {
            if context.renaming != task.id { model.promote(id: task.id) }
        }
        .simultaneousGesture(TapGesture().onEnded {
            if context.renaming != task.id { focus.wrappedValue = .task(task.id) }
        })
        .contextMenu {
            TaskMenuItems(task: task, isCurrent: false)
        }
        .onDrag { drag.provider(for: task.id) }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(task.title)
        .accessibilityIdentifier("task-\(task.id)")
    }

    private var background: some ShapeStyle {
        style == .boardSide ? AnyShapeStyle(.quaternary.opacity(0.6)) : AnyShapeStyle(.clear)
    }
}

/// A bucket's tasks, with the line a drop would land on, or the word
/// `empty`. Every row is a drag source, and the list a drop target.
struct BucketList: View {
    @Environment(QueueModel.self) private var model
    @Environment(DragState.self) private var drag
    let bucket: Bucket
    let tasks: [QueueTask]
    let style: RowStyle
    var focus: FocusState<FocusTarget?>.Binding
    /// Board: the room under the last row is part of the target, so a drop
    /// there lands at the end.
    var minHeight: CGFloat = 0
    @State private var frames: [UInt64: CGRect] = [:]

    private var space: String { "list-\(BucketName.of(bucket))" }

    var body: some View {
        if tasks.isEmpty {
            Text("empty")
                .foregroundStyle(.tertiary)
                .frame(maxWidth: .infinity, minHeight: max(minHeight, 28), alignment: style == .boardSide || style == .boardNext ? .center : .leading)
                .padding(.horizontal, 8)
                .contentShape(Rectangle())
                .dropRing(.placeholder(bucket), in: drag)
                .appendDrop(bucket, empty: true, ring: .placeholder(bucket), drag: drag, model: model)
                .accessibilityIdentifier("empty-\(BucketName.of(bucket).lowercased())")
        } else {
            VStack(spacing: 0) {
                VStack(alignment: .leading, spacing: style.gap) {
                ForEach(tasks, id: \.id) { task in
                    TaskRow(task: task, style: style, focus: focus)
                        .rowFrame(task.id, in: space)
                        .overlay(alignment: .top) {
                            if drag.mark == .before(task.id) {
                                InsertionLine().offset(y: -style.gap / 2 - 1)
                            }
                        }
                        .overlay(alignment: .bottom) {
                            if drag.mark == .end(bucket), task.id == tasks.last?.id {
                                InsertionLine().offset(y: style.gap / 2 + 1)
                            }
                        }
                }
                }
                .accessibilityElement(children: .contain)
                .accessibilityLabel(BucketName.of(bucket))
                .accessibilityIdentifier("list-\(BucketName.of(bucket).lowercased())")
                // The room under the rows, which takes a drop at the end.
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, minHeight: minHeight, alignment: .top)
            .contentShape(Rectangle())
            .coordinateSpace(name: space)
            .onPreferenceChange(RowFrames.self) { frames = $0 }
            .onDrop(of: [.queueFocusTask], delegate: ListDropDelegate(
                bucket: bucket, rows: tasks.map(\.id), frames: frames, drag: drag, model: model
            ))
        }
    }
}

/// A bucket's name and how many tasks it holds. A drop on it lands at the
/// end of the bucket.
struct BucketHeader: View {
    @Environment(QueueModel.self) private var model
    @Environment(DragState.self) private var drag
    let bucket: Bucket
    let count: Int

    var body: some View {
        HStack(spacing: 6) {
            Text(BucketName.of(bucket))
                .font(.headline)
            Text("\(count)")
                .foregroundStyle(.secondary)
                .monospacedDigit()
            Spacer()
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 2)
        .contentShape(Rectangle())
        .dropRing(.heading(bucket), in: drag, cornerRadius: 6)
        .appendDrop(bucket, empty: count == 0, ring: .heading(bucket), drag: drag, model: model)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
        .accessibilityIdentifier("heading-\(BucketName.of(bucket).lowercased())")
    }
}

// MARK: The current task

/// The current task's panel: a banner at the top of the Queue, a card in
/// the Board's Now quadrant.
struct NowPanel: View {
    enum Style { case banner, card }

    @Environment(QueueModel.self) private var model
    @Environment(DragState.self) private var drag
    @Environment(WindowContext.self) private var context
    let style: Style
    var focus: FocusState<FocusTarget?>.Binding

    var body: some View {
        let current = model.snapshot.current
        content(current)
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .background(TagStyle.accent(current?.tag).opacity(current?.tag == nil ? 0.08 : 0.16),
                        in: RoundedRectangle(cornerRadius: 10))
            .overlay {
                if let current, focus.wrappedValue == .task(current.id) {
                    RoundedRectangle(cornerRadius: 10)
                        .strokeBorder(Color(nsColor: .keyboardFocusIndicatorColor), lineWidth: 2)
                }
            }
            .dropRing(.now, in: drag, cornerRadius: 10)
            .onDrop(of: [.queueFocusTask], delegate: promoteDrop)
            .modifier(CurrentTaskFocus(current: current, renaming: context.renaming != nil && context.renaming == current?.id,
                                       focus: focus, drag: drag))
            .accessibilityElement(children: .contain)
            .accessibilityLabel(current.map { "Now: \($0.title)" } ?? "Now: empty")
            .accessibilityIdentifier("now-panel")
    }

    /// A task dropped on the panel takes over as the current task.
    private var promoteDrop: WholeDropDelegate {
        WholeDropDelegate(mark: .ring(.now), drag: drag) { id in
            model.promote(id: id)
            return true
        }
    }

    @ViewBuilder
    private func content(_ current: QueueTask?) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                if style == .banner {
                    SectionHeading("NOW")
                }
                if let current {
                    tagButton(current)
                }
                Spacer()
                if let current, let secs = model.elapsed(of: current) {
                    Button {
                        model.togglePause()
                    } label: {
                        Text(longElapsed(secs: secs))
                            .font(.body.monospacedDigit())
                            .foregroundStyle(current.pausedAt == nil ? .secondary : .tertiary)
                    }
                    .buttonStyle(.borderless)
                    .help(current.pausedAt == nil ? "Pause" : "Resume")
                    .accessibilityLabel(current.pausedAt == nil ? "Pause, \(longElapsed(secs: secs))" : "Resume, \(longElapsed(secs: secs))")
                    .accessibilityIdentifier("now-timer")
                }
            }
            if let current {
                title(current)
                HStack(spacing: 8) {
                    if style == .banner {
                        Button {
                            model.complete(id: current.id)
                        } label: {
                            Image(systemName: "checkmark")
                        }
                        .help("Done")
                        .accessibilityLabel("Done")
                        .accessibilityIdentifier("now-done")
                    } else {
                        Button("Done") { model.complete(id: current.id) }
                            .accessibilityIdentifier("now-done")
                    }
                    Spacer()
                    TaskMenuButton(task: current, isCurrent: true)
                }
                .buttonStyle(.borderless)
            } else {
                Text(style == .banner ? "empty — promote one ↑" : "empty — drop a task here")
                    .foregroundStyle(.secondary)
            }
        }
    }

    /// The tag cycles when clicked. On the Board it is always there, and
    /// reads `–` until the task has one.
    @ViewBuilder
    private func tagButton(_ current: QueueTask) -> some View {
        if let tag = current.tag {
            Button { model.cycleTag(id: current.id) } label: { TagChip(tag: tag) }
                .buttonStyle(.plain)
                .help("Change the tag")
        } else if style == .card {
            Button { model.cycleTag(id: current.id) } label: {
                Text("–")
                    .font(.caption2.weight(.bold))
                    .padding(.horizontal, 5)
                    .overlay(RoundedRectangle(cornerRadius: 4).strokeBorder(.secondary))
            }
            .buttonStyle(.plain)
            .help("Add a tag")
            .accessibilityLabel("No tag")
        }
    }

    @ViewBuilder
    private func title(_ current: QueueTask) -> some View {
        if context.renaming == current.id {
            RenameField(task: current, focus: focus, font: style == .card ? .title2 : .title3)
        } else if style == .banner {
            TruncatedTitle(text: current.title, lines: 3, font: .title3.weight(.semibold))
        } else {
            // The Board shows the whole title, scrolling when it is long.
            ScrollView {
                Text(current.title)
                    .font(.title2.weight(.semibold))
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(minHeight: 60, maxHeight: 196)
            .fixedSize(horizontal: false, vertical: true)
            // A scroll view takes drops for itself, so it passes them on.
            .onDrop(of: [.queueFocusTask], delegate: promoteDrop)
        }
    }
}

/// The panel takes the keyboard and can be dragged only while it holds a
/// task: an empty panel is no stop and nothing to pick up.
private struct CurrentTaskFocus: ViewModifier {
    let current: QueueTask?
    /// While renamed, the field inside takes the keyboard instead.
    let renaming: Bool
    var focus: FocusState<FocusTarget?>.Binding
    let drag: DragState

    func body(content: Content) -> some View {
        if let current {
            content
                .contentShape(Rectangle())
                .focusable(!renaming)
                .focused(focus, equals: .task(current.id))
                .focusEffectDisabled()
                .simultaneousGesture(TapGesture().onEnded { focus.wrappedValue = .task(current.id) })
                .contextMenu { TaskMenuItems(task: current, isCurrent: true) }
                .onDrag { drag.provider(for: current.id) }
        } else {
            content
        }
    }
}
