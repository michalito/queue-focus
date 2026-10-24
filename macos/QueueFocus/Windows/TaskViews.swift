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
    /// Whether the keyboard's ring shows: once a key has moved it, and not
    /// after a click, as GTK shows focus only to someone using the keys.
    var showsFocus = false

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

    /// The space between a row's parts. Side's cards are the roomier ones.
    var gap: CGFloat {
        self == .boardSide ? 10 : 8
    }

    /// The space between one row and the next: Side's cards stand apart,
    /// the rest touch.
    var spacing: CGFloat {
        self == .boardSide ? 8 : 0
    }

    var padding: EdgeInsets {
        switch self {
        case .queue: EdgeInsets(top: 8, leading: 12, bottom: 8, trailing: 12)
        case .later, .boardLater: EdgeInsets(top: 6, leading: 0, bottom: 6, trailing: 0)
        case .boardSide: EdgeInsets(top: 10, leading: 14, bottom: 10, trailing: 12)
        case .boardNext: EdgeInsets(top: 8, leading: 12, bottom: 8, trailing: 8)
        }
    }

    /// Stepped back: grey titles in Later, either view.
    var dim: Bool {
        self == .later || self == .boardLater
    }
}

// MARK: Titles

/// A title cut to `lines`, with the whole of it in a tooltip only when it
/// was cut: a title that fits shows no tooltip.
struct TruncatedTitle: View {
    let text: String
    let lines: Int
    var font: Font = .body
    /// Stepped back, as the Board's Later rows are: grey, still readable.
    var dim = false
    @State private var shownHeight: CGFloat = 0
    @State private var fullHeight: CGFloat = 0

    var body: some View {
        Text(text)
            .font(font)
            .foregroundStyle(dim ? Adw.dim : Adw.foreground)
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

/// The task menu, in a row's context menu and behind its menu button,
/// with GNOME's items in GNOME's order.
struct TaskMenuItems: View {
    @Environment(QueueModel.self) private var model
    @Environment(WindowContext.self) private var context
    let task: QueueTask
    let isCurrent: Bool

    var body: some View {
        if !isCurrent {
            Button("Make current") { model.promote(id: task.id) }.disabled(!model.focusAvailable)
        }
        Button("Cycle tag") { model.cycleTag(id: task.id) }
        Button("Done") { model.complete(id: task.id) }.disabled(isCurrent && !model.focusAvailable)
        Divider()
        // Moving to Now is "Make current", which leads the menu already.
        ForEach([Bucket.next, .side, .later].filter { $0 != task.bucket }, id: \.self) { bucket in
            Button("Move to \(BucketName.of(bucket))") { model.move(id: task.id, to: bucket) }
                .disabled(isCurrent && !model.focusAvailable)
        }
        Divider()
        Button("Rename…") { context.beginRename(task) }
        Button("Delete", role: .destructive) { model.remove(id: task.id) }.disabled(isCurrent && !model.focusAvailable)
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

/// The `⋮` button that opens the task menu: faded until the pointer or the
/// keyboard is on its row.
struct TaskMenuButton: View {
    let task: QueueTask
    let isCurrent: Bool
    var size: CGFloat = 24
    var resting: Double = 0.45

    var body: some View {
        // Named: a menu takes its name from its title, and the glyph alone
        // says nothing.
        Menu {
            TaskMenuItems(task: task, isCurrent: isCurrent)
        } label: {
            Label { Text("Task menu") } icon: { MoreGlyph(size: size) }
                .labelStyle(.iconOnly)
        }
        .menuStyle(.button)
        .buttonStyle(.adwIcon(resting: resting))
        .menuIndicator(.hidden)
        .fixedSize()
        .help("Menu")
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
            .modifier(AdwEntry(font: font, padding: EdgeInsets(top: 1, leading: 6, bottom: 1, trailing: 6), focused: true))
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
                // rename selects the old title to type over it. Select All
                // goes to the key window, so only when that is this one: a
                // rename that moved with its task in a window behind keeps
                // its caret, and another window's field keeps its text.
                DispatchQueue.main.async {
                    focus.wrappedValue = .rename(task.id)
                    guard activeState == .key else { return }
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

/// One task in a list: its title, its tag, and what can be done with it.
struct TaskRow: View {
    @Environment(QueueModel.self) private var model
    @Environment(\.displayOptions) private var options
    @Environment(DragState.self) private var drag
    @Environment(WindowContext.self) private var context
    let task: QueueTask
    let style: RowStyle
    var focus: FocusState<FocusTarget?>.Binding
    @State private var hovering = false

    var body: some View {
        let focused = focus.wrappedValue == .task(task.id) && context.showsFocus
        // The row's buttons come up under the pointer and the keyboard.
        let resting = hovering || focused ? 1 : 0.45
        HStack(spacing: style.gap) {
            // Side's cards lead with the tag: they are the ones read out of
            // order.
            if style == .boardSide, let tag = task.tag {
                TagChip(tag: tag)
            }
            Group {
                if context.renaming == task.id {
                    RenameField(task: task, focus: focus)
                } else {
                    // Later steps back by its title alone: a dimmed chip
                    // would no longer read.
                    TruncatedTitle(text: task.title, lines: style.lines,
                                   font: style == .boardSide ? Adw.font(1.1, .bold) : Adw.font(),
                                   dim: style.dim && !options.increaseContrast)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if style != .boardSide, let tag = task.tag {
                TagChip(tag: tag)
            }
            if style.inlineButtons {
                Button {
                    model.promote(id: task.id)
                } label: {
                    IconGlyph(systemName: "arrow.up.to.line")
                }
                .buttonStyle(.adwIcon(resting: resting))
                .help("Make current (⏎)")
                .accessibilityLabel("Make current")
                .disabled(!model.focusAvailable)
                .accessibilityIdentifier("promote-\(task.id)")
                if style == .later {
                    Button("→ next") { model.move(id: task.id, to: .next) }
                        .font(Adw.font(0.72, .bold))
                        // GNOME fades it to 0.55, under 4.5 to 1 for its
                        // words; 0.75 reads.
                        .buttonStyle(AdwButtonStyle(padding: EdgeInsets(top: 0, leading: 8, bottom: 0, trailing: 8),
                                                    restingOpacity: 0.75))
                        .frame(height: 22)
                        .overlay(RoundedRectangle(cornerRadius: Adw.radius).strokeBorder(Adw.cardShade))
                        .help("Move to Next")
                        .accessibilityIdentifier("to-next-\(task.id)")
                }
            }
            TaskMenuButton(task: task, isCurrent: false, resting: resting)
        }
        .padding(style.padding)
        .background(background, in: RoundedRectangle(cornerRadius: style == .boardSide ? Adw.cardRadius : 0))
        .overlay {
            if style == .boardSide {
                RoundedRectangle(cornerRadius: Adw.cardRadius).strokeBorder(Adw.cardShade)
            }
        }
        .overlay {
            if focused {
                FocusRing(cornerRadius: style == .boardSide ? Adw.cardRadius : Adw.radius)
            }
        }
        .opacity(drag.fades(task.id) ? 0.35 : 1)
        .contentShape(Rectangle())
        .onHover { hovering = $0 }
        // While renamed, the field inside takes the keyboard instead.
        .focusable(context.renaming != task.id)
        .focused(focus, equals: .task(task.id))
        .focusEffectDisabled()
        .onTapGesture(count: 2) {
            if context.renaming != task.id { model.promote(id: task.id) }
        }
        .simultaneousGesture(TapGesture().onEnded {
            if context.renaming != task.id {
                context.showsFocus = false
                focus.wrappedValue = .task(task.id)
            }
        })
        .contextMenu {
            TaskMenuItems(task: task, isCurrent: false)
        }
        .onDrag { drag.provider(for: task.id) }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(task.title)
        .accessibilityIdentifier("task-\(task.id)")
    }

    @Environment(\.adwAccent) private var accent

    private var background: Color {
        switch style {
        case .boardSide: hovering ? Adw.cardHover : accent.sideCard
        case .queue, .boardNext: hovering ? Adw.wash(0.04) : .clear
        case .later, .boardLater: .clear
        }
    }
}

/// Where the keyboard is: Adwaita's ring, the accent at half strength, or
/// the accent's ink round the current task's panel, as the app's stylesheet
/// has it.
struct FocusRing: View {
    var cornerRadius: CGFloat = Adw.radius
    var strong = false
    @Environment(\.adwAccent) private var accent

    var body: some View {
        RoundedRectangle(cornerRadius: cornerRadius)
            .strokeBorder(strong ? accent.ink : accent.background.opacity(0.5), lineWidth: 2)
            .accessibilityHidden(true)
    }
}

/// A bucket's tasks, with the line a drop would land on, or the word
/// `empty`. Every row is a drag source, and the list a drop target.
struct BucketList: View {
    @Environment(QueueModel.self) private var model
    @Environment(DragState.self) private var drag
    @Environment(WindowContext.self) private var context
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
            empty
                .contentShape(Rectangle())
                .dropRing(.placeholder(bucket), in: drag, page: context.page,
                          cornerRadius: style == .boardSide ? Adw.cardRadius : Adw.radius)
                .appendDrop(bucket, empty: true, ring: .placeholder(bucket), drag: drag, page: context.page, model: model)
                .accessibilityIdentifier("empty-\(BucketName.of(bucket).lowercased())")
        } else {
            VStack(spacing: 0) {
                VStack(alignment: .leading, spacing: style.spacing) {
                    ForEach(Array(tasks.enumerated()), id: \.element.id) { index, task in
                        VStack(spacing: 0) {
                            // Next's rows are split by hairlines, the first
                            // against the card's edge drawing none; each of
                            // Later's has a faint one above.
                            if style == .boardNext, index > 0 {
                                Hairline()
                            } else if style == .boardLater {
                                Hairline(color: Adw.wash(0.06))
                            }
                            TaskRow(task: task, style: style, focus: focus)
                        }
                        .rowFrame(task.id, in: space)
                        .overlay(alignment: .top) {
                            if drag.marks(.before(task.id), in: context.page) {
                                InsertionLine().offset(y: style == .boardSide ? -5 : -1)
                                    .padding(.horizontal, style == .boardNext ? 8 : 0)
                            }
                        }
                        .overlay(alignment: .bottom) {
                            if drag.marks(.end(bucket), in: context.page), task.id == tasks.last?.id {
                                InsertionLine().offset(y: style == .boardSide ? 5 : 1)
                                    .padding(.horizontal, style == .boardNext ? 8 : 0)
                            }
                        }
                    }
                }
                // The room under the rows, which takes a drop at the end.
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, minHeight: minHeight, alignment: .top)
            .contentShape(Rectangle())
            .coordinateSpace(name: space)
            .onPreferenceChange(RowFrames.self) { frames = $0 }
            .onDrop(of: [.queueFocusTask], delegate: ListDropDelegate(
                page: context.page, bucket: bucket, rows: tasks.map(\.id), frames: frames, drag: drag, model: model
            ))
        }
    }

    @ViewBuilder
    private var empty: some View {
        switch style {
        case .boardSide:
            // Side's is a box of its own rather than a line of text.
            EmptyLabel()
                .frame(maxWidth: .infinity, minHeight: max(minHeight, 70))
                .overlay(RoundedRectangle(cornerRadius: Adw.cardRadius).strokeBorder(Adw.wash(0.1)))
        case .queue:
            EmptyLabel()
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(EdgeInsets(top: 6, leading: 12, bottom: 10, trailing: 12))
        case .boardNext:
            EmptyLabel()
                .frame(maxWidth: .infinity, minHeight: minHeight, alignment: .topLeading)
                .padding(EdgeInsets(top: 10, leading: 12, bottom: 10, trailing: 12))
        case .later, .boardLater:
            EmptyLabel()
                .frame(maxWidth: .infinity, minHeight: minHeight, alignment: .topLeading)
                .padding(.vertical, style == .boardLater ? 8 : 6)
        }
    }
}

extension View {
    /// A bucket's heading and its rows, as one group VoiceOver reads by the
    /// bucket's name. The heading belongs inside: SwiftUI drops a group's
    /// only element, so a bucket holding one task would lose its row.
    func bucketGroup(_ bucket: Bucket) -> some View {
        accessibilityElement(children: .contain)
            .accessibilityLabel(BucketName.of(bucket))
            .accessibilityIdentifier("list-\(BucketName.of(bucket).lowercased())")
    }
}

/// A bucket's name and how many tasks it holds. A drop on it lands at the
/// end of the bucket.
struct BucketHeader: View {
    @Environment(QueueModel.self) private var model
    @Environment(DragState.self) private var drag
    @Environment(WindowContext.self) private var context
    let bucket: Bucket
    let count: Int
    var insets = EdgeInsets()

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(BucketName.of(bucket))
                .bucketHeaderStyle()
                .opacity(0.75)
            SectionCount(count: count)
            Spacer()
        }
        .padding(insets)
        .contentShape(Rectangle())
        .dropRing(.heading(bucket), in: drag, page: context.page, cornerRadius: Adw.radius)
        .appendDrop(bucket, empty: count == 0, ring: .heading(bucket), drag: drag, page: context.page, model: model)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
        .accessibilityIdentifier("heading-\(BucketName.of(bucket).lowercased())")
    }
}

// MARK: The current task

/// The current task's panel: a band across the top of the Queue, a card in
/// the Board's Now quadrant. The loudest thing on the page.
struct NowPanel: View {
    enum Style { case banner, card }

    @Environment(QueueModel.self) private var model
    @Environment(DragState.self) private var drag
    @Environment(WindowContext.self) private var context
    @Environment(\.adwAccent) private var accent
    let style: Style
    var focus: FocusState<FocusTarget?>.Binding
    @State private var hovering = false

    var body: some View {
        let current = model.snapshot.current
        let focused = context.showsFocus && current.map { focus.wrappedValue == .task($0.id) } ?? false
        content(current, raised: hovering || focused)
            .padding(style == .banner ? EdgeInsets(top: 18, leading: 20, bottom: 18, trailing: 20)
                                      : EdgeInsets(top: 14, leading: 18, bottom: 12, trailing: 18))
            .frame(maxWidth: .infinity, minHeight: style == .card ? 112 : nil, alignment: .topLeading)
            .background(background(current))
            .overlay {
                if focused {
                    FocusRing(cornerRadius: style == .card ? Adw.cardRadius : 0, strong: true)
                }
            }
            .dropRing(.now, in: drag, page: context.page, cornerRadius: style == .card ? Adw.cardRadius : 0)
            .onDrop(of: [.queueFocusTask], delegate: promoteDrop)
            .onHover { hovering = $0 }
            .modifier(CurrentTaskFocus(current: current, renaming: context.renaming != nil && context.renaming == current?.id,
                                       focus: focus, drag: drag, clicked: { context.showsFocus = false }))
            .accessibilityElement(children: .contain)
            .accessibilityLabel(current.map { "Now: \($0.title)" } ?? "Now: empty")
            .accessibilityIdentifier("now-panel")
    }

    @ViewBuilder
    private func background(_ current: QueueTask?) -> some View {
        switch style {
        case .banner:
            // An empty banner has no tint, only the hairline under it.
            VStack(spacing: 0) {
                (current == nil ? Color.clear : accent.panel)
                Hairline()
            }
        case .card:
            RoundedRectangle(cornerRadius: Adw.cardRadius)
                .fill(accent.panel)
                .overlay(RoundedRectangle(cornerRadius: Adw.cardRadius).strokeBorder(Adw.cardShade))
        }
    }

    /// A task dropped on the panel takes over as the current task.
    private var promoteDrop: WholeDropDelegate {
        WholeDropDelegate(page: context.page, mark: .ring(.now), drag: drag) { id in
            model.promote(id: id)
            return true
        }
    }

    @ViewBuilder
    private func content(_ current: QueueTask?, raised: Bool) -> some View {
        if let current {
            VStack(alignment: .leading, spacing: style == .card ? 8 : 6) {
                HStack(spacing: 8) {
                    if style == .banner {
                        Text("Now").bucketHeaderStyle().fontWeight(.heavy).foregroundStyle(accent.ink)
                            .accessibilityAddTraits(.isHeader)
                    }
                    tagButton(current)
                    Spacer(minLength: 0)
                    timer(current)
                }
                switch style {
                case .banner:
                    // One band: the title with its buttons beside it.
                    HStack(spacing: 8) {
                        title(current)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        Button {
                            model.complete(id: current.id)
                        } label: {
                            IconGlyph(systemName: "checkmark", size: 28)
                        }
                        .buttonStyle(.adwIcon(resting: raised ? 1 : 0.55))
                        .help("Done — delete and pull the next task (d)")
                        .accessibilityLabel("Done")
                        .disabled(!model.focusAvailable)
                        .accessibilityIdentifier("now-done")
                        TaskMenuButton(task: current, isCurrent: true, size: 28, resting: raised ? 1 : 0.55)
                    }
                case .card:
                    // The Board spells the title out large, over a line of
                    // actions.
                    title(current)
                        .frame(maxHeight: .infinity, alignment: .top)
                    HStack(spacing: 6) {
                        Button {
                            model.complete(id: current.id)
                        } label: {
                            HStack(spacing: 8) {
                                Image(systemName: "checkmark").font(.system(size: 12, weight: .bold))
                                Text("Done")
                            }
                            .font(Adw.font(0.85, .bold))
                            .frame(minHeight: 28)
                        }
                        .buttonStyle(AdwButtonStyle(kind: .filled, padding: EdgeInsets(top: 0, leading: 12, bottom: 0, trailing: 12)))
                        .help("Done — delete and pull the next task (d)")
                        .disabled(!model.focusAvailable)
                        .accessibilityIdentifier("now-done")
                        Spacer()
                        TaskMenuButton(task: current, isCurrent: true, size: 28, resting: raised ? 1 : 0.55)
                    }
                    .padding(.top, 4)
                }
            }
        } else {
            VStack(alignment: .leading, spacing: 6) {
                // The Board's heading names the quadrant already.
                if style == .banner {
                    Text("Now").bucketHeaderStyle().opacity(0.75)
                        .accessibilityAddTraits(.isHeader)
                }
                EmptyLabel(text: style == .banner ? "empty — promote one ↑" : "empty — drop a task here",
                           color: style == .card ? accent.dim : Adw.dim)
                    .frame(maxWidth: .infinity, maxHeight: style == .card ? .infinity : nil,
                           alignment: style == .card ? .center : .leading)
            }
            .frame(minHeight: style == .card ? 86 : nil)
        }
    }

    /// The clock, which pauses and resumes. The glyph keeps its room so the
    /// time never moves: shown while paused, and under the pointer.
    @ViewBuilder
    private func timer(_ current: QueueTask) -> some View {
        if let secs = model.elapsed(of: current) {
            let paused = current.pausedAt != nil
            TimerButton(time: longElapsed(secs: secs), paused: paused) {
                model.togglePause()
            }
            .disabled(!model.focusAvailable)
        }
    }

    /// The tag cycles when clicked. On the Board it is always there, and
    /// reads `–` until the task has one.
    @ViewBuilder
    private func tagButton(_ current: QueueTask) -> some View {
        if let tag = current.tag {
            Button { model.cycleTag(id: current.id) } label: { TagChip(tag: tag) }
                .buttonStyle(.plain)
                .help("Cycle tag (t)")
        } else if style == .card {
            Button { model.cycleTag(id: current.id) } label: { UntaggedChip(color: accent.dim) }
                .buttonStyle(.plain)
                .help("Cycle tag (t)")
                .accessibilityLabel("No tag")
        }
    }

    @ViewBuilder
    private func title(_ current: QueueTask) -> some View {
        if context.renaming == current.id {
            RenameField(task: current, focus: focus, font: Adw.font(1.15, .heavy))
        } else if style == .banner {
            TruncatedTitle(text: current.title, lines: 3, font: Adw.font(1.3, .heavy))
        } else {
            // The Board shows the whole title, scrolling when it is long.
            ScrollView {
                Text(current.title)
                    .font(Adw.font(1.5, .heavy))
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(minHeight: 60, maxHeight: 196)
            .fixedSize(horizontal: false, vertical: true)
            // A scroll view takes drops for itself, so it passes them on.
            .onDrop(of: [.queueFocusTask], delegate: promoteDrop)
        }
    }
}

/// The current task's clock as a button: the time in the accent, in
/// monospace digits.
private struct TimerButton: View {
    let time: String
    let paused: Bool
    let toggle: () -> Void
    @Environment(\.adwAccent) private var accent
    @State private var hovering = false

    var body: some View {
        Button(action: toggle) {
            HStack(spacing: 6) {
                Image(systemName: paused ? "play.fill" : "pause.fill")
                    .font(.system(size: 11, weight: .bold))
                    .opacity(paused || hovering ? 0.7 : 0)
                // Paused, the time steps back to grey, and the glyph says
                // why: not a fainter colour alone.
                Text(time)
                    .font(Adw.font(1.1, .semibold, design: .monospaced))
                    .monospacedDigit()
                    .foregroundStyle(paused ? accent.dim : accent.ink)
            }
        }
        .buttonStyle(AdwButtonStyle(padding: EdgeInsets(top: 1, leading: 7, bottom: 1, trailing: 7), pressedWash: 0.07))
        .onHover { hovering = $0 }
        .help(paused ? "Resume (p)" : "Pause (p)")
        .accessibilityLabel(paused ? "Resume, \(time)" : "Pause, \(time)")
        .accessibilityIdentifier("now-timer")
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
    let clicked: () -> Void

    func body(content: Content) -> some View {
        if let current {
            content
                .contentShape(Rectangle())
                .focusable(!renaming)
                .focused(focus, equals: .task(current.id))
                .focusEffectDisabled()
                // A click in the rename field is someone placing the caret.
                .simultaneousGesture(TapGesture().onEnded {
                    if !renaming {
                        clicked()
                        focus.wrappedValue = .task(current.id)
                    }
                })
                .contextMenu { TaskMenuItems(task: current, isCurrent: true) }
                .onDrag { drag.provider(for: current.id) }
        } else {
            content
        }
    }
}
