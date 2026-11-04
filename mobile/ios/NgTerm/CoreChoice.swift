import NgtermCore

#if DEBUG
    import NgtermCoreFakes
#endif

/// Which core the app runs on: the linked Rust core, or, in a Debug build launched with ``CoreChoice/fakeArgument``
/// (the UI tests), the fake one.
internal struct CoreChoice {
    /// The launch argument that selects the fake core in a Debug build.
    internal static let fakeArgument = "-ngterm-fake-core"

    /// The version of the chosen core.
    internal let version: String

    internal init(arguments: [String]) {
        #if DEBUG
            if arguments.contains(Self.fakeArgument) {
                version = FakeCoreInfo.version
                return
            }
        #endif
        version = CoreInfo.version
    }
}
