import SwiftUI
import UniformTypeIdentifiers

extension UTType {
    /// A task being dragged inside Queue Focus. It never leaves the app, and
    /// nothing else's drag is mistaken for one.
    static let queueFocusTask = UTType(exportedAs: "org.queuefocus.task-id")
}

/// What a drop would do, drawn while the pointer is over a target.
enum DropMark: Equatable {
    /// A line in front of this row.
    case before(UInt64)
    /// A line after the last row of the bucket.
    case end(Bucket)
    /// A ring around a target that takes the task whole.
    case ring(DropRing)
}

/// The targets that take a task whole rather than at a place in a list.
enum DropRing: Equatable {
    /// The current task's panel, or the Board's Now heading: promote.
    case now
    /// A bucket's heading while the bucket is empty: append.
    case heading(Bucket)
    /// A bucket's "empty" line: append.
    case placeholder(Bucket)
    /// The Queue's Later shelf toggle, open or not: append.
    case laterShelf
}

/// The one drag in progress, shared by every window so a task can be dragged
/// between them.
@MainActor
@Observable
final class DragState {
    /// The task picked up. A drag that ends outside our windows says nothing,
    /// so this is only trusted while one of our targets is under the pointer.
    var pickedUp: UInt64?
    /// The one mark on screen, as GTK keeps one, and the window it is in:
    /// both windows can show the same task, and only the one under the
    /// pointer draws it.
    private var mark: (page: Page, mark: DropMark?)?
    /// One of our targets is under the pointer.
    private(set) var over = false

    /// The row the task came from fades while it is over a target.
    func fades(_ id: UInt64) -> Bool {
        over && pickedUp == id
    }

    /// Whether `page` draws `mark` now.
    func marks(_ mark: DropMark, in page: Page) -> Bool {
        self.mark?.page == page && self.mark?.mark == mark
    }

    func show(_ mark: DropMark?, in page: Page) {
        over = true
        self.mark = (page, mark)
    }

    /// The pointer left a target in `page`, or dropped on it. A window it
    /// has already entered keeps its mark.
    func clear(in page: Page) {
        guard mark == nil || mark?.page == page else { return }
        over = false
        mark = nil
    }

    /// Start dragging task `id`. The id is recorded now, at pickup, so every
    /// target knows what is coming without loading anything from the drag.
    func provider(for id: UInt64) -> NSItemProvider {
        pickedUp = id
        let provider = NSItemProvider()
        provider.registerDataRepresentation(
            forTypeIdentifier: UTType.queueFocusTask.identifier,
            visibility: .ownProcess
        ) { completion in
            completion(Data(String(id).utf8), nil)
            return nil
        }
        return provider
    }

    func accepts(_ info: DropInfo) -> Bool {
        pickedUp != nil && info.hasItemsConforming(to: [.queueFocusTask])
    }
}

/// The frames of a list's rows, in the list's coordinate space.
struct RowFrames: PreferenceKey {
    static let defaultValue: [UInt64: CGRect] = [:]

    static func reduce(value: inout [UInt64: CGRect], nextValue: () -> [UInt64: CGRect]) {
        value.merge(nextValue()) { $1 }
    }
}

extension View {
    /// Report this row's frame to the list it is in.
    func rowFrame(_ id: UInt64, in space: String) -> some View {
        background(GeometryReader { geometry in
            Color.clear.preference(key: RowFrames.self, value: [id: geometry.frame(in: .named(space))])
        })
    }
}

/// A drop on a bucket's rows: in front of the row under the pointer, or at
/// the end of the bucket.
struct ListDropDelegate: DropDelegate {
    let page: Page
    let bucket: Bucket
    /// The rows' ids, top to bottom.
    let rows: [UInt64]
    let frames: [UInt64: CGRect]
    let drag: DragState
    let model: QueueModel

    func validateDrop(info: DropInfo) -> Bool {
        drag.accepts(info)
    }

    func dropEntered(info: DropInfo) {
        update(info)
    }

    func dropUpdated(info: DropInfo) -> DropProposal? {
        update(info)
        return DropProposal(operation: .move)
    }

    func dropExited(info: DropInfo) {
        drag.clear(in: page)
    }

    func performDrop(info: DropInfo) -> Bool {
        defer { drag.clear(in: page) }
        guard let id = drag.pickedUp else { return false }
        return model.move(id: id, to: bucket, before: anchor(info))
    }

    private func anchor(_ info: DropInfo) -> UInt64? {
        let placed = rows.compactMap { id in frames[id].map { (id: id, frame: $0) } }
        return Placement.anchor(at: info.location.y, rows: placed)
    }

    private func update(_ info: DropInfo) {
        guard let anchor = anchor(info) else {
            drag.show(.end(bucket), in: page)
            return
        }
        // Over the row it came from a drop changes nothing: the row fades,
        // and no line promises otherwise.
        drag.show(anchor == drag.pickedUp ? nil : .before(anchor), in: page)
    }
}

/// A drop on something that takes the task whole: the current task's panel,
/// a heading, an "empty" line, the Later shelf.
struct WholeDropDelegate: DropDelegate {
    let page: Page
    /// What is drawn while the pointer is over it.
    let mark: DropMark
    let drag: DragState
    /// What the drop does with the task; whether it took it.
    let perform: @MainActor (UInt64) -> Bool

    func validateDrop(info: DropInfo) -> Bool {
        drag.accepts(info)
    }

    func dropEntered(info: DropInfo) {
        drag.show(mark, in: page)
    }

    func dropUpdated(info: DropInfo) -> DropProposal? {
        drag.show(mark, in: page)
        return DropProposal(operation: .move)
    }

    func dropExited(info: DropInfo) {
        drag.clear(in: page)
    }

    func performDrop(info: DropInfo) -> Bool {
        defer { drag.clear(in: page) }
        guard let id = drag.pickedUp else { return false }
        return perform(id)
    }
}

extension View {
    /// A ring around this view while a drop on it would take the task.
    func dropRing(_ ring: DropRing, in drag: DragState, page: Page, cornerRadius: CGFloat = 8) -> some View {
        overlay {
            if drag.marks(.ring(ring), in: page) {
                RoundedRectangle(cornerRadius: cornerRadius)
                    .strokeBorder(Color(nsColor: .controlAccentColor), lineWidth: 2)
                    .accessibilityHidden(true)
            }
        }
    }

    /// Take a dropped task to the end of `bucket`. The mark is a line after
    /// the last row while the bucket has rows, and `ring` while it has none.
    func appendDrop(_ bucket: Bucket, empty: Bool, ring: DropRing, drag: DragState, page: Page, model: QueueModel) -> some View {
        onDrop(of: [.queueFocusTask], delegate: WholeDropDelegate(
            page: page,
            mark: empty ? .ring(ring) : .end(bucket),
            drag: drag,
            perform: { model.move(id: $0, to: bucket, before: nil) }
        ))
    }
}

/// The line a drop would land on.
struct InsertionLine: View {
    var body: some View {
        Capsule()
            .fill(Color(nsColor: .controlAccentColor))
            .frame(height: 2)
            .accessibilityHidden(true)
    }
}
