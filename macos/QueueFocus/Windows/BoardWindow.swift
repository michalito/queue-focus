import SwiftUI

/// The Board view: the same tasks in four quadrants. Now takes the wide top
/// left, Side the narrow top right, Next the wide bottom left and Later the
/// narrow bottom right.
struct BoardWindow: View {
    @Environment(QueueModel.self) private var model

    var body: some View {
        let snapshot = model.snapshot
        Grid(horizontalSpacing: 12, verticalSpacing: 12) {
            GridRow {
                NowBanner(current: snapshot.current)
                    .frame(maxHeight: .infinity, alignment: .top)
                    .gridCellColumns(2)
                Quadrant(title: "Side", tasks: snapshot.side, empty: "Nothing on the side")
            }
            .frame(maxHeight: 220)
            GridRow {
                Quadrant(title: "Next", tasks: snapshot.next, empty: "Nothing queued")
                    .gridCellColumns(2)
                Quadrant(title: "Later", tasks: snapshot.later, empty: "Nothing for later")
            }
        }
        .padding(12)
        .frame(minWidth: 720, minHeight: 480)
        .navigationTitle("Board")
    }
}

private struct Quadrant: View {
    let title: String
    let tasks: [QueueTask]
    let empty: String

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            SectionHeading(title.uppercased())
            List {
                TaskRows(tasks: tasks, empty: empty)
            }
            .listStyle(.bordered)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
