#if NGTERM_FAKES
    /// An upload a ``FakeMobileCore`` stored: the machine it went to, the file's name and how many bytes it held.
    public struct FakeUpload: Equatable, Sendable {
        /// The machine's id.
        public let machineId: String
        /// The file's name, as the app gave it.
        public let name: String
        /// How many bytes it held.
        public let size: Int

        /// The upload of `size` bytes named `name` to `machineId`.
        public init(machineId: String, name: String, size: Int) {
            self.machineId = machineId
            self.name = name
            self.size = size
        }
    }
#endif
