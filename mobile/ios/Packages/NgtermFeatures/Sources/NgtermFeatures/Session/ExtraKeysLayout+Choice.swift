import NgtermPlatform
import NgtermTerminal

extension ExtraKeysLayout {
    /// The layout Settings' `choice` asks for.
    internal init(_ choice: ExtraKeysChoice) {
        switch choice {
        case .fit:
            self = .fit

        case .twoLines:
            self = .twoLines

        case .hidden:
            self = .hidden
        }
    }
}
