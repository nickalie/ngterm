import Foundation
import NgtermCore

/// What went wrong, in words a banner can show.
internal enum ErrorWords {
    /// `error` in words: the core's reason for a key error, an error's own description when it has one (a Keychain
    /// refusal's status words, a store's reason), and Foundation's words only for the rest, which for a type that isn't
    /// a `LocalizedError` say no more than its name.
    internal static func of(_ error: any Error) -> String {
        switch error {
        case let error as KeyError:
            error.reason

        case let error as CustomStringConvertible:
            error.description

        default:
            error.localizedDescription
        }
    }
}
