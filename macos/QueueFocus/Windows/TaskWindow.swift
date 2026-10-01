import SwiftUI

/// What the Queue and the Board share: the add field, the task keys, focus
/// that always has somewhere to be, the shortcuts popover, the message line,
/// and the current task's colour.
struct TaskWindow<Content: View>: View {
    @Environment(QueueModel.self) private var model
    @Environment(\.openWindow) private var openWindow
    @Environment(\.dismissWindow) private var dismissWindow
    @State private var context: WindowContext
    @FocusState private var focus: FocusTarget?
    @State private var memory: FocusMemory?
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
        let tag = model.snapshot.current?.tag
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                WindowAddField(focus: $focus, firstStop: { order.first })
                WindowButtons(page: page)
            }
            .padding([.horizontal, .top], 12)
            .padding(.bottom, 8)
            content($focus)
            MessageLine()
        }
        .environment(context)
        .background(TagStyle.accent(tag).opacity(tag == nil ? 0 : 0.04))
        .tint(tag.map { TagStyle.accent($0) } ?? .accentColor)
        .onKeyPress(phases: .down, action: handle)
        .onAppear { settleFocus() }
        .onChange(of: model.snapshot) { _, _ in
            if let id = context.renaming, model.task(id) == nil {
                context.renaming = nil
            }
            settleFocus(reclaim: abandoned)
        }
        .onChange(of: context.laterOpen) { _, _ in settleFocus() }
        .onChange(of: focus) { _, now in
            if case .task(let id) = now {
                memory = FocusMemory(order: order, id: id)
            } else if now == nil || abandoned {
                settleFocus(reclaim: true)
            }
        }
    }

    /// The add field holds the keyboard only because the task that had it
    /// went: AppKit hands a vanished view's focus to the first field. That
    /// is not someone choosing to type.
    private var abandoned: Bool {
        guard focus == .add, let memory else { return false }
        return !order.contains(memory.id)
    }

    /// Focus is never nowhere: the keys act on the focused task, and with
    /// nothing focused they would do nothing at all. It goes back to the task
    /// it was on, or to whatever now sits where that was, or to the add field.
    /// `reclaim` takes it back from the add field too.
    private func settleFocus(reclaim: Bool = false) {
        // A rename holds the keyboard until it ends.
        guard context.renaming == nil else { return }
        switch focus {
        case .add where !reclaim, .rename:
            return
        case .task(let id) where order.contains(id):
            memory = FocusMemory(order: order, id: id)
            return
        default:
            break
        }
        if let id = memory?.restore(in: order) ?? order.first {
            focus = .task(id)
        } else {
            focus = .add
        }
    }

    private func handle(_ press: KeyPress) -> KeyPress.Result {
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

/// The other view, the keys, and Settings, beside the add field. Not in the
/// toolbar: a toolbar rebuilt with every tick of the clock sends AppKit's
/// layout round in circles until it gives up.
private struct WindowButtons: View {
    @Environment(WindowContext.self) private var context
    @Environment(\.openWindow) private var openWindow
    let page: Page

    var body: some View {
        @Bindable var context = context
        HStack(spacing: 2) {
            Button(page == .queue ? "Board" : "Queue") {
                openWindow(id: page == .queue ? WindowID.board : WindowID.queue)
            }
            .help(page == .queue ? "Open the Board (⌘2)" : "Open the Queue (⌘1)")
            .accessibilityIdentifier("open-other-view")
            Button {
                context.showShortcuts.toggle()
            } label: {
                Image(systemName: "questionmark.circle")
            }
            .help("Keyboard shortcuts (?)")
            .accessibilityLabel("Keyboard shortcuts")
            .popover(isPresented: $context.showShortcuts) { ShortcutList() }
            .accessibilityIdentifier("shortcuts")
            SettingsLink {
                Image(systemName: "gearshape")
            }
            .help("Settings (⌘,)")
            .accessibilityLabel("Settings")
        }
        .buttonStyle(.borderless)
    }
}

/// The window's add field. Return adds where Settings says; Command-Return
/// adds the task as the current one. Escape clears a draft, then hands the
/// keyboard back to the tasks.
private struct WindowAddField: View {
    @Environment(QueueModel.self) private var model
    var focus: FocusState<FocusTarget?>.Binding
    let firstStop: () -> UInt64?
    @State private var draft = ""

    var body: some View {
        TextField("Add…  !now  #w #p  @later @side", text: $draft)
            .textFieldStyle(.roundedBorder)
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
            .accessibilityIdentifier("window-add-field")
    }

    private func submit(asCurrent: Bool) {
        if model.add(draft, asCurrent: asCurrent) {
            draft = ""
        }
    }
}

/// The keys, as the `?` button lists them.
private struct ShortcutList: View {
    var body: some View {
        Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 6) {
            ForEach(shortcutList, id: \.keys) { shortcut in
                GridRow {
                    Text(shortcut.keys)
                        .font(.body.monospaced())
                        .foregroundStyle(.secondary)
                    Text(shortcut.does)
                }
            }
        }
        .padding(14)
    }
}
