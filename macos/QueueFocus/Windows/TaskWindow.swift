import SwiftUI

/// What the Queue and the Board share: the add field, the task keys, focus
/// that always has somewhere to be, the shortcuts popover, the message line,
/// and the current task's colour.
struct TaskWindow<Content: View>: View {
    @Environment(QueueModel.self) private var model
    @Environment(\.displayOptions) private var options
    @Environment(\.openWindow) private var openWindow
    @Environment(\.dismissWindow) private var dismissWindow
    @State private var context: WindowContext
    @FocusState private var focus: FocusTarget?
    @State private var keeper = FocusKeeper()
    private let content: (FocusState<FocusTarget?>.Binding) -> Content

    init(page: Page, @ViewBuilder content: @escaping (FocusState<FocusTarget?>.Binding) -> Content) {
        _context = State(initialValue: WindowContext(page: page))
        self.content = content
    }

    private var page: Page { context.page }

    private var order: [UInt64] {
        Placement.visibleOrder(model.snapshot, page: page, laterOpen: context.laterOpen)
    }

    var body: some View {
        let accent = AdwAccent.of(model.snapshot.current?.tag)
        VStack(spacing: 0) {
            // GNOME's header bar and the add field's band under it, tinted
            // together by the current task's tag.
            VStack(spacing: 0) {
                HeaderBar {
                    ViewSwitcher(page: page)
                } end: {
                    WindowButtons()
                }
                WindowAddField(focus: $focus, firstStop: { order.first })
                    .padding(10)
            }
            .background(accent.headerBar)
            MessageLine()
            content($focus)
        }
        .background(Adw.window)
        .foregroundStyle(Adw.foreground)
        .modifier(HeaderBarWindow())
        .environment(context)
        .environment(\.adwAccent, accent)
        .tint(accent.background)
        .onKeyPress(phases: .down, action: handle)
        .onAppear { settleFocus() }
        .onChange(of: model.snapshot) { _, _ in
            keepRenameInView()
            settleFocus()
        }
        .onChange(of: context.laterOpen) { _, _ in
            // Closing the shelf over a rename ends it.
            if let id = context.renaming, !order.contains(id) {
                context.renaming = nil
            }
            settleFocus()
        }
        .onChange(of: focus) { _, now in
            // Tab moves the keyboard without a key reaching `handle`.
            if NSApp.currentEvent?.type == .keyDown { context.showsFocus = true }
            if let target = keeper.moved(to: now, order: order, renaming: context.renaming) {
                focus = target
            }
        }
    }

    private func settleFocus() {
        if let target = keeper.settle(focus, order: order, renaming: context.renaming) {
            focus = target
        }
    }

    /// A rename follows its task: it ends when the task goes, and when
    /// something elsewhere moves the task to Later, the shelf opens on it.
    private func keepRenameInView() {
        guard let id = context.renaming else { return }
        guard let task = model.task(id) else {
            context.renaming = nil
            return
        }
        if !order.contains(id), page == .queue, task.bucket == .later {
            context.laterOpen = true
        }
    }

    private func handle(_ press: KeyPress) -> KeyPress.Result {
        context.showsFocus = true
        // Typing goes to the field being typed in.
        switch focus {
        case .add, .rename: return .ignored
        default: break
        }
        let focused: UInt64? = if case .task(let id) = focus { id } else { nil }
        guard let action = KeyMap.action(for: Keystroke(press), focused: focused) else {
            return .ignored
        }
        perform(action)
        return .handled
    }

    private func perform(_ action: KeyAction) {
        switch action {
        case .close:
            dismissWindow(id: page == .queue ? WindowID.queue : WindowID.board)
        case .focusAdd:
            focus = .add
        case .show(let other):
            if other != page {
                openWindow(id: other == .queue ? WindowID.queue : WindowID.board)
            }
        case .toggleLater:
            if page == .queue { context.laterOpen.toggle() }
        case .focusStep(let delta):
            let current: UInt64? = if case .task(let id) = focus { id } else { nil }
            if let next = Placement.step(from: current, by: delta, in: order) {
                focus = .task(next)
            }
        case .togglePause:
            model.togglePause()
        case .toggleShortcuts:
            context.showShortcuts.toggle()
        case .rename(let id):
            if let task = model.task(id) { context.beginRename(task) }
        case .shift(let id, let delta):
            model.shift(id: id, by: Int32(delta))
        case .complete(let id):
            model.complete(id: id)
        case .cycleTag(let id):
            model.cycleTag(id: id)
        case .promote(let id):
            model.promote(id: id)
        case .move(let id, let bucket):
            model.move(id: id, to: bucket)
        }
    }
}

/// Queue and Board, as GNOME's header bar switches between them. Here they
/// are two windows: the other one opens, and this one stays.
private struct ViewSwitcher: View {
    @Environment(\.openWindow) private var openWindow
    @Environment(\.adwAccent) private var accent
    let page: Page

    var body: some View {
        Segmented(options: [(Page.queue, "Queue"), (Page.board, "Board")], selection: page,
                  help: { $0 == .queue ? "Queue (q, ⌘1)" : "Board (b, ⌘2)" },
                  identifier: { $0 == page ? nil : "open-other-view" },
                  surface: accent.headerBarNS) { other in
            if other != page {
                openWindow(id: other == .queue ? WindowID.queue : WindowID.board)
            }
        }
    }
}

/// Settings and the keys, at the end of the header bar. Not in a toolbar:
/// a toolbar rebuilt with every tick of the clock sends AppKit's layout
/// round in circles until it gives up.
private struct WindowButtons: View {
    @Environment(WindowContext.self) private var context
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        @Bindable var context = context
        HStack(spacing: 6) {
            HeaderGlyphButton(glyph: "gearshape.fill", symbol: true) { openWindow(id: WindowID.settings) }
                .help("Settings (⌘,)")
                .accessibilityLabel("Settings")
            HeaderGlyphButton(glyph: "?") { context.showShortcuts.toggle() }
                .help("Keyboard shortcuts (?)")
                .accessibilityLabel("Keyboard shortcuts")
                .popover(isPresented: $context.showShortcuts, arrowEdge: .bottom) { ShortcutList() }
                .accessibilityIdentifier("shortcuts")
        }
    }
}

/// The window's add field. Return adds where Settings says, or to Now while
/// it is empty; Command-Return adds the task as the current one. Escape
/// clears a draft, then hands the keyboard back to the tasks.
private struct WindowAddField: View {
    @Environment(QueueModel.self) private var model
    var focus: FocusState<FocusTarget?>.Binding
    let firstStop: () -> UInt64?
    @State private var draft = ""
    @State private var pendingAdd: PendingAdd?

    var body: some View {
        TextField("Add…  !now  #w #p  @later @side  (⏎)", text: $draft)
                .disabled(pendingAdd != nil)
                .onChange(of: model.focusResults) { _, _ in reconcilePendingAdd() }
                .onAppear { reconcilePendingAdd() }
            .modifier(AdwEntry(focused: focus.wrappedValue == .add))
            .focused(focus, equals: .add)
            .onSubmit {
                submit(asCurrent: NSApp.currentEvent?.modifierFlags.contains(.command) == true)
            }
            .onKeyPress(.return, phases: .down) { press in
                guard press.modifiers.contains(.command) else { return .ignored }
                submit(asCurrent: true)
                return .handled
            }
            .onKeyPress(.escape) {
                if draft.isEmpty {
                    focus.wrappedValue = firstStop().map(FocusTarget.task)
                } else {
                    draft = ""
                }
                return .handled
            }
            .onChange(of: draft) { _, text in
                let limit = Int(maxTitleChars())
                if text.count > limit { draft = String(text.prefix(limit)) }
            }
            .accessibilityLabel("Add a task")
            .accessibilityIdentifier("window-add-field")
    }

    private func submit(asCurrent: Bool) {
        guard pendingAdd == nil else { return }
        if model.add(draft, asCurrent: asCurrent) {
            draft = ""
        } else if let command = model.lastRequestPending {
            pendingAdd = PendingAdd(command: command, text: draft)
            reconcilePendingAdd()
        }
    }

    private func reconcilePendingAdd() {
        guard let pending = pendingAdd, let result = model.focusResults[pending.command] else { return }
        pendingAdd = nil
        if result.applied && draft == pending.text {
            draft = ""
        }
    }
}

/// The keys, as the `?` button lists them.
private struct ShortcutList: View {
    var body: some View {
        ShortcutLines(rowSpacing: 5)
            .font(Adw.font(0.8))
            .padding(.horizontal, 14)
            .padding(.vertical, 12)
    }
}

/// The task keys, a line each: the key in a faded monospace column, then
/// what it does. One element a line, which VoiceOver reads whole.
struct ShortcutLines: View {
    let rowSpacing: CGFloat

    var body: some View {
        Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: rowSpacing) {
            ForEach(shortcutList, id: \.keys) { shortcut in
                GridRow {
                    Text(shortcut.keys)
                        .font(Adw.font(0.8, .semibold, design: .monospaced))
                        .foregroundStyle(Adw.dim)
                        .frame(minWidth: 44, alignment: .leading)
                    Text(shortcut.does)
                }
                .accessibilityElement(children: .combine)
            }
        }
    }
}
