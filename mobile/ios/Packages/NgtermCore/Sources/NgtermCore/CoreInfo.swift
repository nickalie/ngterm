import NgtermCoreBindings

/// What the app knows about the Rust core it links.
public enum CoreInfo {
    /// The version of the linked core: the ngterm workspace version it was built at (`core_version()`).
    public static var version: String {
        coreVersion()
    }
}
