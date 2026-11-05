/// One of a setting's values as Settings offers it: its title, and what it means when that needs saying.
internal struct SettingsChoice<Value: Hashable & Sendable>: Hashable, Sendable {
    internal let value: Value
    internal let title: String
    internal let note: String?

    internal init(_ value: Value, _ title: String, note: String?) {
        self.value = value
        self.title = title
        self.note = note
    }
}
