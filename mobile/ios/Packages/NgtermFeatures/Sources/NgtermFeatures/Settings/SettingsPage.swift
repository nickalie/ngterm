/// A screen Settings pushes.
internal enum SettingsPage: Hashable {
    /// Importing a key.
    case importKey
    /// The public half of the key with this id, and switching the machines to it.
    case publicKey(String)
}
