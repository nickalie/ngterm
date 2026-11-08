import Testing

@testable import NgtermFeatures

/// The machine form's sheet identity: Add, and Edit of each machine.
@Suite("the machine form's targets")
internal struct FormTargetTests {
    @Test("the machine form's sheet is one per target: Add, and Edit of each machine")
    internal func identity() {
        let targets: [FormTarget] = [.add, .edit("m1"), .edit("m2"), .edit("add")]
        #expect(Set(targets.map(\.id)).count == targets.count)
        let editing = FormTarget.edit("m1")
        #expect(editing.id == FormTarget.edit("m1").id)
    }
}
