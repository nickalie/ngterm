/// A setting with a fixed set of values, stored by its case's name so that the stored form doesn't depend on the
/// order of the cases; a name this build doesn't know reads as none.
public protocol StoredChoice: CaseIterable, Hashable, Sendable {}

extension StoredChoice {
    /// The value's name in the store: its case's name.
    public var storedName: String {
        "\(self)"
    }

    /// The value stored as `name`; `nil` for none, or for one this build doesn't know.
    public static func stored(_ name: String?) -> Self? {
        allCases.first { $0.storedName == name }
    }
}
