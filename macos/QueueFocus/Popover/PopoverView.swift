import SwiftUI

/// What the popover asks of whoever shows it.
struct PopoverActions {
    var close: () -> Void
    var quit: () -> Void
}

/// The menu bar's popover, drawn as GNOME's top bar menu is (the
/// extension's `stylesheet.css`): dark, as the Shell's menus are, washes of
/// white over it, the current task and its actions on the left, the add
/// field and Side on the right. Every title is one line, shown whole: the
/// columns start at the menu's widths and grow with the titles in them, up
/// to the width of the screen. While the popover is open it only grows, so
/// a button stays where the pointer left it when a task goes.
struct PopoverView: View {
    static let columns: (left: CGFloat, right: CGFloat) = (250, 348)
    static let spacing: CGFloat = 14
    static let padding: CGFloat = 8
    static let minimumWidth = padding + columns.left + spacing + columns.right + padding

    @Environment(QueueModel.self) private var model
    @Environment(PopoverWidths.self) private var widths
    let actions: PopoverActions

    var body: some View {
        VStack(spacing: 0) {
            MenuColumns(spacing: Self.spacing,
                        floors: [max(Self.columns.left, widths.left), max(Self.columns.right, widths.right)],
                        cap: widths.cap - 2 * Self.padding) {
                FocusColumn(actions: actions)
                    .frame(maxHeight: .infinity, alignment: .top)
                    .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { widths.grow(left: $0) }
                QueueColumn(actions: actions)
                    .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { widths.grow(right: $0) }
            }
            .padding(Self.padding)
            .padding(.vertical, 4)
            // A message wraps in the width the columns set.
            MessageLine(showsSync: false)
                .frame(idealWidth: 0, maxWidth: .infinity)
        }
        .frame(minWidth: min(Self.minimumWidth, widths.cap))
        .fixedSize(horizontal: true, vertical: false)
        .foregroundStyle(.white)
        .background(Shell.background)
        .environment(\.colorScheme, .dark)
    }
}

/// The widths the popover's columns have reached while it is open, which
/// they do not go back below until it closes.
@MainActor
@Observable
final class PopoverWidths {
    private(set) var left: CGFloat = 0
    private(set) var right: CGFloat = 0
    /// As wide as the popover may be: the screen, less a margin.
    var cap: CGFloat = 1200

    func grow(left width: CGFloat) {
        if width > left + 0.5 { left = width.rounded(.up) }
    }

    func grow(right width: CGFloat) {
        if width > right + 0.5 { right = width.rounded(.up) }
    }

    /// The popover closed: it opens next time at what its titles need.
    func reset() {
        left = 0
        right = 0
    }
}

/// The menu's two columns, side by side, each as wide as its titles need
/// and no narrower than its floor. Past `cap` the wider column gives way
/// first, and its titles are cut short.
private struct MenuColumns: Layout {
    let spacing: CGFloat
    let floors: [CGFloat]
    let cap: CGFloat

    private func widths(_ subviews: Subviews) -> [CGFloat] {
        let gaps = spacing * CGFloat(max(0, subviews.count - 1))
        // On a screen too narrow for even the floors, they shrink alike.
        let total = floors.reduce(0, +)
        let floors = total + gaps > cap ? floors.map { $0 * max(0, cap - gaps) / total } : floors
        var widths = subviews.indices.map { max(floors[$0], subviews[$0].sizeThatFits(.unspecified).width) }
        var excess = widths.reduce(0, +) + gaps - cap
        while excess > 0.5,
              let widest = widths.indices.filter({ widths[$0] > floors[$0] }).max(by: { widths[$0] < widths[$1] }) {
            let cut = min(excess, widths[widest] - floors[widest])
            widths[widest] -= cut
            excess -= cut
        }
        return widths
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let widths = widths(subviews)
        let height = subviews.indices.map { subviews[$0].sizeThatFits(ProposedViewSize(width: widths[$0], height: nil)).height }.max() ?? 0
        return CGSize(width: widths.reduce(0, +) + spacing * CGFloat(max(0, subviews.count - 1)), height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var x = bounds.minX
        for (subview, width) in zip(subviews, widths(subviews)) {
            subview.place(at: CGPoint(x: x, y: bounds.minY), anchor: .topLeading,
                          proposal: ProposedViewSize(width: width, height: bounds.height))
            x += width + spacing
        }
    }
}

/// The Shell menu's colours: white at some strength, and the tags'.
private enum Shell {
    /// The Shell's menu, as Ubuntu 24.04 draws it.
    static let background = Color(red: 0x35 / 255, green: 0x35 / 255, blue: 0x35 / 255)
    static func white(_ opacity: Double) -> Color { Color.white.opacity(opacity) }
    static let orange = Color(red: 0xFF / 255, green: 0xA3 / 255, blue: 0x48 / 255)

    /// The card's and the Side cards' wash: the tag's accent.
    static func wash(_ tag: TaskTag?, _ strength: Double) -> Color {
        switch tag {
        case .work: Color(red: 53 / 255, green: 132 / 255, blue: 228 / 255).opacity(strength)
        case .personal: Color(red: 230 / 255, green: 97 / 255, blue: 0).opacity(strength)
        case nil: white(strength == 0.16 ? 0.06 : 0.05)
        }
    }
}

/// A button in the menu's clothes: a wash of white, stronger under the
/// pointer.
private struct MenuButtonStyle: ButtonStyle {
    var fill: Color? = nil
    var hoverFill: Color? = nil
    var rest: Double = 0.1
    var hover: Double = 0.18
    var radius: CGFloat = 8
    var minHeight: CGFloat = 38
    var padding: CGFloat = 10

    func makeBody(configuration: Configuration) -> some View {
        StyledButton(configuration: configuration, style: self)
    }

    private struct StyledButton: View {
        let configuration: Configuration
        let style: MenuButtonStyle
        @Environment(\.isEnabled) private var isEnabled
        @State private var hovering = false

        var body: some View {
            let lit = hovering || configuration.isPressed
            configuration.label
                .frame(minHeight: style.minHeight)
                .padding(.horizontal, style.padding)
                .background(fill(lit), in: RoundedRectangle(cornerRadius: style.radius))
                .contentShape(RoundedRectangle(cornerRadius: style.radius))
                .opacity(isEnabled ? 1 : 0.5)
                .onHover { hovering = $0 && isEnabled }
        }

        private func fill(_ lit: Bool) -> Color {
            if let fill = style.fill {
                return lit ? (style.hoverFill ?? fill) : fill
            }
            return Shell.white(lit ? style.hover : style.rest)
        }
    }
}

// MARK: Left: the current task

private struct FocusColumn: View {
    @Environment(QueueModel.self) private var model
    @Environment(\.displayOptions) private var options
    @Environment(\.openWindow) private var openWindow
    let actions: PopoverActions

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            NowCard()
            if let current = model.snapshot.current {
                HStack(spacing: 8) {
                    Button {
                        if let done = model.completeCurrent().task {
                            model.offerUndo(for: done)
                        }
                    } label: {
                        Text("✓ Done").frame(maxWidth: .infinity)
                    }
                    // Done takes the tag's colour, a shade darker than
                    // GNOME's so white on it reads at 4.5 to 1.
                    .buttonStyle(MenuButtonStyle(fill: current.tag.map { TagStyle.solid($0, options) } ?? Shell.white(0.22),
                                                 hoverFill: current.tag.map { TagStyle.solid($0, options).opacity(0.85) } ?? Shell.white(0.3)))
                    .disabled(!model.focusAvailable)
                    .accessibilityLabel("Done")
                    .accessibilityIdentifier("done-current")

                    let paused = current.pausedAt != nil
                    Button {
                        model.togglePause()
                    } label: {
                        Text(paused ? "▶ Resume" : "\(StatusTitle.pauseGlyph) Pause").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(MenuButtonStyle())
                    .disabled(!model.focusAvailable)
                    .accessibilityLabel(paused ? "Resume" : "Pause")
                    .accessibilityIdentifier("pause-current")
                }
                .font(Adw.font(1, .bold))
            }
            if let offer = model.liveUndoOffer {
                UndoRow(offer: offer)
            }
            if let status = model.syncMessage {
                Text(status)
                    .font(.system(size: 13))
                    .foregroundStyle(Color(red: 0xB8 / 255, green: 0xB8 / 255, blue: 0xBD / 255))
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(idealWidth: 0, maxWidth: .infinity, alignment: .leading)
                    .padding(.vertical, 6)
                    .accessibilityIdentifier("sync-status")
            }
            Spacer(minLength: 0)
            // The view buttons along the bottom, under a faint line.
            VStack(spacing: 0) {
                Rectangle().fill(Shell.white(0.08)).frame(height: 1)
                HStack(spacing: 2) {
                    FooterButton(title: "Queue") {
                        openWindow(id: WindowID.queue)
                        actions.close()
                    }
                    .accessibilityIdentifier("open-queue")
                    FooterButton(title: "Board") {
                        openWindow(id: WindowID.board)
                        actions.close()
                    }
                    .accessibilityIdentifier("open-board")
                    Spacer()
                    GearMenu(actions: actions, openSettings: {
                        openWindow(id: WindowID.settings)
                        actions.close()
                    })
                }
                .padding(.top, 10)
            }
        }
    }
}

/// Queue or Board along the menu's foot: faint words that light up.
private struct FooterButton: View {
    let title: String
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Text(title)
                .font(Adw.font(0.92))
                .foregroundStyle(Shell.white(hovering ? 1 : 0.65))
        }
        .buttonStyle(MenuButtonStyle(rest: 0, hover: 0.08, radius: 6, minHeight: 28))
        .onHover { hovering = $0 }
    }
}

/// The current task as a card washed with its tag's accent: NOW, the title,
/// and the clock, large, at the foot.
private struct NowCard: View {
    @Environment(QueueModel.self) private var model

    var body: some View {
        let current = model.snapshot.current
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Text("NOW")
                    .font(Adw.font(0.8, .bold))
                    .foregroundStyle(Shell.white(0.6))
                    .accessibilityAddTraits(.isHeader)
                if let tag = current?.tag {
                    TagChip(tag: tag, compact: true)
                }
                Spacer(minLength: 0)
                if current?.pausedAt != nil {
                    Text("PAUSED")
                        .font(Adw.font(0.8, .bold))
                        .foregroundStyle(Shell.orange)
                }
            }
            if let current {
                Text(current.title)
                    .font(Adw.font(1.16, .bold))
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .help(current.title)
                    .accessibilityIdentifier("now-title")
                Spacer(minLength: 0)
                if let secs = model.elapsed(of: current) {
                    let paused = current.pausedAt != nil
                    let clock = shortElapsed(secs: secs, paused: false)
                    // The card's clock trails the glyph, and is shown whatever
                    // the menu bar's timer setting says.
                    Text(paused ? "\(clock) \(StatusTitle.pauseGlyph)" : clock)
                        .font(Adw.font(2.05, .semibold, design: .monospaced))
                        .monospacedDigit()
                        .foregroundStyle(Shell.white(paused ? 0.55 : 1))
                        .accessibilityHidden(true)
                }
            } else {
                Text("Nothing in Now.\nPick one from Side →\nor add one with !")
                    .font(Adw.font(1.02))
                    .foregroundStyle(Shell.white(0.6))
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 0)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
        .frame(maxWidth: .infinity, minHeight: 190, alignment: .topLeading)
        .background(Shell.wash(current?.tag, 0.16), in: RoundedRectangle(cornerRadius: 12))
        .accessibilityElement(children: .combine)
        .accessibilityValue(clockValue(current))
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
            // The one line that is a report, not a task, arriving under a
            // pointer that has just pressed Done: it takes the width the
            // column has and asks for none, so it cannot move the menu.
            Text("Done · \(offer.title)")
                .font(Adw.font(0.9))
                .foregroundStyle(Shell.white(0.7))
                .lineLimit(1)
                .truncationMode(.tail)
                .help(offer.title)
                .frame(idealWidth: 0, maxWidth: .infinity, alignment: .leading)
            Button("↶ Undo") {
                model.undo()
            }
            .font(Adw.font(1, .bold))
            .buttonStyle(MenuButtonStyle(rest: 0.12, hover: 0.22, radius: 6, minHeight: 28))
            .accessibilityLabel("Undo")
            .accessibilityIdentifier("undo")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background(Shell.white(0.06), in: RoundedRectangle(cornerRadius: 8))
    }
}

private struct GearMenu: View {
    @Environment(LoginItem.self) private var loginItem
    let actions: PopoverActions
    let openSettings: () -> Void

    var body: some View {
        // Titled: a menu takes its name from its title, not from an
        // accessibility label. On the Mac the gear also holds what a menu
        // bar app has no menu bar for.
        Menu {
            Button("Settings…", action: openSettings)
                .accessibilityIdentifier("gear-settings")
            Toggle("Launch at Login", isOn: Binding(get: { loginItem.isEnabled }, set: { loginItem.set($0) }))
            if loginItem.needsApproval {
                Button("Allow in Login Items…", action: loginItem.openSystemSettings)
            }
            Divider()
            Button("Quit Queue Focus", action: actions.quit)
                .accessibilityIdentifier("gear-quit")
        } label: {
            Label("Settings and more", systemImage: "gearshape.fill")
                .labelStyle(.iconOnly)
                .font(.system(size: 13, weight: .bold))
                .foregroundStyle(Shell.white(0.65))
                .frame(minWidth: 28, minHeight: 28)
        }
        .menuStyle(.button)
        .buttonStyle(MenuButtonStyle(rest: 0, hover: 0.08, radius: 6, minHeight: 28, padding: 0))
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
        VStack(alignment: .leading, spacing: 6) {
            AddField(actions: actions)
            Text("SIDE")
                .font(Adw.font(0.8, .bold))
                .tracking(0.08 * 0.8 * Adw.em)
                .foregroundStyle(Shell.white(0.6))
                .padding(EdgeInsets(top: 8, leading: 8, bottom: 4, trailing: 8))
                .accessibilityAddTraits(.isHeader)
            if model.snapshot.side.isEmpty {
                Text("Nothing on the side.\nAdd one with @side.")
                    // 0.55, not GNOME's 0.5, to read at 4.5 to 1.
                    .foregroundStyle(Shell.white(0.55))
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: .infinity, minHeight: 70)
                    .padding(8)
                    .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Shell.white(0.1)))
                    .padding(1)
            } else {
                // The list scrolls past five or so cards.
                ScrollView {
                    VStack(spacing: 8) {
                        ForEach(model.snapshot.side, id: \.id) { task in
                            SideCard(task: task)
                        }
                    }
                    .padding(1)
                }
                .frame(maxHeight: 300)
                .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

/// The add field. Return adds where Settings says, or to Now while it is
/// empty; Command-Return adds the task as the current one. A task added
/// closes the popover, as in GNOME.
private struct AddField: View {
    @Environment(QueueModel.self) private var model
    let actions: PopoverActions
    @State private var draft = ""
    @State private var pendingAdd: PendingAdd?
    @State private var hovering = false
    @FocusState private var focused: Bool

    var body: some View {
        TextField("Add…  !now  #w #p  @later @side", text: $draft)
                .disabled(pendingAdd != nil)
                .onChange(of: model.focusResults) { _, _ in reconcilePendingAdd() }
                .onAppear { reconcilePendingAdd() }
            .textFieldStyle(.plain)
            .font(Adw.font())
            .padding(.horizontal, 12)
            .frame(minHeight: 34)
            .background(Shell.white(hovering ? 0.14 : 0.1), in: RoundedRectangle(cornerRadius: 8))
            .overlay {
                if focused {
                    RoundedRectangle(cornerRadius: 8).strokeBorder(Shell.white(0.5), lineWidth: 2)
                }
            }
            .onHover { hovering = $0 }
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
        guard pendingAdd == nil else { return }
        if model.add(draft, asCurrent: asCurrent) {
            draft = ""
            actions.close()
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
        HStack(spacing: 10) {
            if let tag = task.tag {
                TagChip(tag: tag, compact: true)
            }
            Text(task.title)
                .font(Adw.font(1.09, .bold))
                .lineLimit(1)
                .truncationMode(.tail)
                .help(task.title)
                .frame(maxWidth: .infinity, alignment: .leading)
            HStack(spacing: 4) {
                Button("↑") {
                    model.promote(task)
                }
                .focused($focused, equals: .promote)
                .accessibilityLabel("Make current")
                .disabled(!model.focusAvailable)
                .accessibilityIdentifier("promote-\(task.id)")
                Button("✓") {
                    if model.complete(task) {
                        model.offerUndo(for: task)
                    }
                }
                .focused($focused, equals: .done)
                .accessibilityLabel("Done")
                .accessibilityIdentifier("done-\(task.id)")
            }
            .font(Adw.font(1, .bold))
            .buttonStyle(MenuButtonStyle(rest: 0.14, hover: 0.26, radius: 6, minHeight: 28, padding: 8))
            .frame(minHeight: 24)
            .opacity(hovering || focused != nil ? 1 : 0)
        }
        .padding(EdgeInsets(top: 10, leading: 14, bottom: 10, trailing: 12))
        .background(hovering ? Shell.white(0.12) : Shell.wash(model.snapshot.current?.tag, 0.07),
                    in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Shell.white(0.12)))
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

/// Why the last request failed, that the settings cannot be written, or
/// the latest problem the engine reported.
struct MessageLine: View {
    @Environment(QueueModel.self) private var model
    /// The popover shows the sync line in its own column, as GNOME's menu
    /// does.
    var showsSync = true

    var body: some View {
        if let error = model.actionError {
            message(error, color: .red, systemImage: "exclamationmark.circle") {
                model.actionError = nil
            }
        } else if let problem = model.settingsProblem {
            message(problem, color: .orange, systemImage: "exclamationmark.triangle") {
                model.dismissSettingsProblem()
            }
        } else if let problem = model.problems.last {
            message(problem, color: .orange, systemImage: "exclamationmark.triangle") {
                model.dismissProblems()
            }
        }
        if showsSync, let status = model.syncMessage {
            // GNOME's sync line: dim words under the add field, nothing more.
            Text(status)
                .font(Adw.font(0.92))
                .foregroundStyle(Adw.dim)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 12).padding(.vertical, 4)
                .accessibilityIdentifier("sync-status")
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
