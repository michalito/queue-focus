import Testing
@testable import QueueFocus

@MainActor
@Suite struct DragStateTests {
    @Test func aMarkIsDrawnOnlyInTheWindowUnderThePointer() {
        let drag = DragState()
        drag.show(.ring(.now), in: .board)
        #expect(drag.marks(.ring(.now), in: .board))
        #expect(!drag.marks(.ring(.now), in: .queue), "the Queue's Now panel stays plain")
        drag.show(.before(3), in: .queue)
        #expect(drag.marks(.before(3), in: .queue))
        #expect(!drag.marks(.before(3), in: .board))
        #expect(!drag.marks(.ring(.now), in: .board), "one mark on screen")
    }

    @Test func leavingOneWindowKeepsTheMarkOfTheOneEntered() {
        let drag = DragState()
        _ = drag.provider(for: 3)
        drag.show(.end(.next), in: .board)
        drag.clear(in: .queue)
        #expect(drag.marks(.end(.next), in: .board))
        #expect(drag.fades(3))
        drag.clear(in: .board)
        #expect(!drag.marks(.end(.next), in: .board))
        #expect(!drag.fades(3))
    }
}
