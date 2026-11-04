#if NGTERM_FAKES
    /// What a host asks the user to sign in with: its message and the link in it.
    public struct FakeSignIn: Equatable, Sendable {
        /// A sign-in as Tailscale SSH asks for one: its banner and link (made up).
        public static let tailscale = Self(
            message: "# Tailscale SSH requires an additional check.\n"
                + "# To authenticate, visit: https://login.tailscale.com/a/l1nk",
            url: "https://login.tailscale.com/a/l1nk"
        )

        /// The banner the host sends.
        public let message: String
        /// The link in it.
        public let url: String

        /// A sign-in with `message`, linking to `url`.
        public init(message: String, url: String) {
            self.message = message
            self.url = url
        }
    }
#endif
