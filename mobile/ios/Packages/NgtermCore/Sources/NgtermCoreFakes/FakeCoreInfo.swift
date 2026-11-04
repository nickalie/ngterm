#if NGTERM_FAKES
    /// What the fake core reports about itself, in place of ``NgtermCore/CoreInfo``.
    public enum FakeCoreInfo {
        /// The version the fake core reports: never a real workspace version, so a screen showing it is plainly fake.
        public static let version = "0.0.0-fake"
    }
#endif
