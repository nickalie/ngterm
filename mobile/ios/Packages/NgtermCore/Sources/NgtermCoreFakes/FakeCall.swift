#if NGTERM_FAKES
    /// One call the app made on a ``FakeMobileCore``: the method's name and its arguments, in order.
    public struct FakeCall: Equatable, Sendable, CustomStringConvertible {
        /// The method's name, as Swift spells it: `connect`, `listSessions`.
        public let method: String
        /// The arguments, in order: an absent optional is `String?.none` (or its type's `none`).
        public let args: [any Hashable & Sendable]

        /// The call as `method(args...)`, for a failing test's message.
        public var description: String {
            "\(method)(\(args.map { String(describing: $0) }.joined(separator: ", ")))"
        }

        /// A call of `method` with `args`.
        public init(_ method: String, _ args: [any Hashable & Sendable] = []) {
            self.method = method
            self.args = args
        }

        /// The same method with equal arguments, each compared as its own type.
        public static func == (lhs: Self, rhs: Self) -> Bool {
            lhs.method == rhs.method && lhs.args.count == rhs.args.count
                && zip(lhs.args, rhs.args).allSatisfy { same($0, $1) }
        }

        private static func same(_ lhs: any Hashable, _ rhs: any Hashable) -> Bool {
            AnyHashable(lhs) == AnyHashable(rhs)
        }
    }
#endif
