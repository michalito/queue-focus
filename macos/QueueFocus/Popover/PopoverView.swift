import SwiftUI

/// What the popover asks of whoever shows it.
struct PopoverActions {
    var close: () -> Void
    var quit: () -> Void
}

/// The menu bar's popover, laid out like GNOME's top bar menu: the current
/// task and its actions on the left, the add field and Side on the right.
struct PopoverView: View {
    static let width: CGFloat = 560

    @Environment(QueueModel.self) private var model
    let actions: PopoverActions

    var body: some View {
        VStack(spacing: 0) {
            HStack(alignment: .top, spacing: 14) {
                FocusColumn(actions: actions)
                    .frame(width: 250)
                QueueColumn(actions: actions)
                    .frame(maxWidth: .infinity)
            }
            .padding(14)
            MessageLine()
        }
        .frame(width: Self.width)
    }
}

// MARK: Left: the current task

private struct FocusColumn: View {
    @Environment(QueueModel.self) private var model
    @Environment(\.displayOptions) private var options
    @Environment(\.openWindow) private var openWindow
    @Environment(\.openSettings) private var openSettings
    let actions: PopoverActions

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            NowCard()
            if let current = model.snapshot.current {
                HStack(spacing: 8) {
                    Button {
                        if let done = model.completeCurrent().task {
                            model.offerUndo(for: done)
                        }
                    } label: {
                        Label("Done", systemImage: "checkmark")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    .tint(TagStyle.solid(current.tag, options))
                    .accessibilityIdentifier("done-current")

                    let paused = current.pausedAt != nil
                    Button {
                        model.togglePause()
                    } label: {
                        Label(paused ? "Resume" : "Pause", systemImage: paused ? "play.fill" : "pause.fill")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.bordered)
                    .accessibilityIdentifier("pause-current")
                }
                .controlSize(.large)
            }
            if let offer = model.liveUndoOffer {
                UndoRow(offer: offer)
            }
            Spacer(minLength: 0)
            Divider()
            HStack(spacing: 2) {
                Button("Queue") {
                    openWindow(id: WindowID.queue)
                    actions.close()
                }
                .accessibilityIdentifier("open-queue")
                Button("Board") {
                    openWindow(id: WindowID.board)
                    actions.close()
                }
                .accessibilityIdentifier("open-board")
                Spacer()
                GearMenu(actions: actions, openSettings: {
                    openSettings()
                    actions.close()
                })
            }
            .buttonStyle(.borderless)
        }
    }
}

/// The current task as a card washed with its tag's accent: NOW, the title,
/// and the clock.
private struct NowCard: View {
    @Environment(QueueModel.self) private var model

    var body: some View {
        let current = model.snapshot.current
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                SectionHeading("NOW")
                if let tag = current?.tag {
                    TagChip(tag: tag)
                }
                Spacer()
                if current?.pausedAt != nil {
                    Text("PAUSED")
                        .font(.caption2.weight(.bold))
                        .foregroundStyle(.orange)
                }
            }
            if let current {
                Text(current.title)
                    .font(.title3.weight(.semibold))
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .help(current.title)
                    .accessibilityIdentifier("now-title")
                if let secs = model.elapsed(of: current) {
                    let paused = current.pausedAt != nil
                    let clock = shortElapsed(secs: secs, paused: false)
                    // The card's clock trails the glyph, and is shown whatever
                    // the menu bar's timer setting says.
                    Text(paused ? "\(clock) \(StatusTitle.pauseGlyph)" : clock)
                        .font(.body.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .accessibilityHidden(true)
                }
            } else {
                Text("Nothing in Now.\nPick one from Side →\nor add one with !")
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, minHeight: 96, alignment: .topLeading)
        .background(cardWash(current?.tag), in: RoundedRectangle(cornerRadius: 10))
        .accessibilityElement(children: .combine)
        .accessibilityValue(clockValue(current))
    }

    private func cardWash(_ tag: TaskTag?) -> some ShapeStyle {
        tag == nil ? AnyShapeStyle(.quaternary.opacity(0.5)) : AnyShapeStyle(TagStyle.accent(tag).opacity(0.16))
    }

    private func clockValue(_ current: QueueTask?) -> String {
        guard let current, let secs = model.elapsed(of: current) else { return "" }
        let clock = shortElapsed(secs: secs, paused: false)
        return current.pausedAt != nil ? "\(clock), paused" : clock
    }
}

/// `Done · title` with an Undo button, while the completion can be undone.
private struct UndoRow: View {
    @Environment(QueueModel.self) private var model
    let offer: UndoOffer

    var body: some View {
        HStack(spacing: 8) {
            Text("Done · \(offer.title)")
                .font(.callout)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.tail)
                .help(offer.title)
            Spacer(minLength: 4)
            Button("Undo") {
                model.undo()
            }
            .accessibilityIdentifier("undo")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 8))
    }
}

private struct GearMenu: View {
    @Environment(LoginItem.self) private var loginItem
    let actions: PopoverActions
    let openSettings: () -> Void

    var body: some View {
        // Titled, and drawn as its symbol alone: a menu takes its name from
        // its title, not from an accessibility label.
        Menu("Settings and more", systemImage: "gearshape") {
            Button("Settings…", action: openSettings)
                .accessibilityIdentifier("gear-settings")
            Toggle("Launch at Login", isOn: Binding(get: { loginItem.isEnabled }, set: { loginItem.set($0) }))
            if loginItem.needsApproval {
                Button("Allow in Login Items…", action: loginItem.openSystemSettings)
            }
            Divider()
            Button("Quit Queue Focus", action: actions.quit)
                .accessibilityIdentifier("gear-quit")
        }
        .labelStyle(.iconOnly)
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .accessibilityIdentifier("gear-menu")
    }
}

// MARK: Right: adding, and Side

private struct QueueColumn: View {
    @Environment(QueueModel.self) private var model
    let actions: PopoverActions

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            AddField(actions: actions)
            SectionHeading("SIDE")
            if model.snapshot.side.isEmpty {
                Text("Nothing on the side.\nAdd one with @side.")
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 18)
            } else {
                ScrollView {
                    VStack(spacing: 6) {
                        ForEach(model.snapshot.side, id: \.id) { task in
                            SideCard(task: task)
                        }
                    }
                }
                .frame(maxHeight: 300)
                .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

/// The add field. Return adds where Settings says; Command-Return adds the
/// task as the current one. A task added closes the popover, as in GNOME.
private struct AddField: View {
    @Environment(QueueModel.self) private var model
    let actions: PopoverActions
    @State private var draft = ""
    @FocusState private var focused: Bool

    var body: some View {
        TextField("Add…  !now  #w #p  @later @side", text: $draft)
            .textFieldStyle(.roundedBorder)
            .controlSize(.large)
            .focused($focused)
            .onSubmit {
                let asCurrent = NSApp.currentEvent?.modifierFlags.contains(.command) == true
                submit(asCurrent: asCurrent)
            }
            .onKeyPress(.return, phases: .down) { press in
                guard press.modifiers.contains(.command) else { return .ignored }
                submit(asCurrent: true)
                return .handled
            }
            .onAppear { focused = true }
            .accessibilityLabel("Add a task")
            .accessibilityIdentifier("add-field")
    }

    private func submit(asCurrent: Bool) {
        if model.add(draft, asCurrent: asCurrent) {
            draft = ""
            actions.close()
        }
    }
}

/// A Side task. Its actions are always laid out, and shown while the
/// pointer or the keyboard is on it, so revealing them moves nothing.
private struct SideCard: View {
    @Environment(QueueModel.self) private var model
    let task: QueueTask
    @State private var hovering = false
    @FocusState private var focused: Action?

    private enum Action {
        case promote, done
    }

    var body: some View {
        HStack(spacing: 6) {
            if let tag = task.tag {
                TagChip(tag: tag)
            }
            Text(task.title)
                .lineLimit(1)
                .truncationMode(.tail)
                .help(task.title)
                .frame(maxWidth: .infinity, alignment: .leading)
            HStack(spacing: 2) {
                Button {
                    model.promote(task)
                } label: {
                    Image(systemName: "arrow.up")
                }
                .focused($focused, equals: .promote)
                .accessibilityLabel("Make current")
                .accessibilityIdentifier("promote-\(task.id)")
                Button {
                    if model.complete(task) {
                        model.offerUndo(for: task)
                    }
                } label: {
                    Image(systemName: "checkmark")
                }
                .focused($focused, equals: .done)
                .accessibilityLabel("Done")
                .accessibilityIdentifier("done-\(task.id)")
            }
            .buttonStyle(.borderless)
            .opacity(hovering || focused != nil ? 1 : 0)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background(TagStyle.accent(model.snapshot.current?.tag).opacity(0.07), in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(.quaternary))
        .onHover { hovering = $0 }
        .accessibilityElement(children: .contain)
        // The buttons show only under the pointer or the keyboard, as in
        // GNOME, and a hidden button is gone for VoiceOver too: the card
        // offers what they do.
        .accessibilityAction(named: "Make current") { model.promote(task) }
        .accessibilityAction(named: "Done") {
            if model.complete(task) {
                model.offerUndo(for: task)
            }
        }
        .accessibilityIdentifier("side-\(task.id)")
    }
}

// MARK: Shared

struct SectionHeading: View {
    let text: String

    init(_ text: String) {
        self.text = text
    }

    var body: some View {
        Text(text)
            .font(.caption.weight(.bold))
            .foregroundStyle(.secondary)
            .accessibilityAddTraits(.isHeader)
    }
}

/// Why the last request failed, or the latest problem the engine reported.
struct MessageLine: View {
    @Environment(QueueModel.self) private var model

    var body: some View {
        if let error = model.actionError {
            message(error, color: .red, systemImage: "exclamationmark.circle") {
                model.actionError = nil
            }
        } else if let problem = model.problems.last {
            message(problem, color: .orange, systemImage: "exclamationmark.triangle") {
                model.dismissProblems()
            }
        }
    }

    private func message(_ text: String, color: Color, systemImage: String, dismiss: @escaping () -> Void) -> some View {
        VStack(spacing: 0) {
            Divider()
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Image(systemName: systemImage)
                    .foregroundStyle(color)
                Text(text)
                    .font(.callout)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .textSelection(.enabled)
                Button {
                    dismiss()
                } label: {
                    Image(systemName: "xmark")
                }
                .buttonStyle(.borderless)
                .accessibilityLabel("Dismiss")
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
        }
        // One group for VoiceOver: the message and what dismisses it.
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("message-line")
    }
}
