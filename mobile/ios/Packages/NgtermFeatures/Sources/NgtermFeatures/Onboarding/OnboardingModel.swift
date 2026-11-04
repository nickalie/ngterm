import Foundation
import NgtermCore
import Observation

/// The first run (mobile/android's `OnboardingViewModel`): the app needs a key before it can reach any machine.
///
/// It starts on the welcome when the app has no key, and done when it has one. Making or reading a key shows the step
/// busy at once, then waits for the key.
@preconcurrency
@MainActor
@Observable
public final class OnboardingModel {
    /// Where the first run is.
    public private(set) var step: OnboardingStep
    @ObservationIgnored private let keys: AppKeys

    /// The first run over `keys`.
    public init(keys: AppKeys) {
        self.keys = keys
        step = keys.keys.isEmpty ? .start : .done
    }

    // what went wrong, in words
    private static func describe(_ error: any Error) -> String {
        switch error {
        case let error as KeyError:
            error.reason

        case let error as CustomStringConvertible:
            error.description

        default:
            error.localizedDescription
        }
    }

    /// Makes a new key on the device; nothing happens unless the welcome is up and idle.
    public func create() async {
        guard case .welcome(false, _) = step else {
            return
        }
        step = .welcome(isBusy: true, failure: nil)
        do {
            step = .keyReady(try await keys.generate())
        } catch {
            step = .welcome(isBusy: false, failure: Self.describe(error))
        }
    }

    /// Opens the import.
    public func startImport() {
        step = .importing(isBusy: false, problem: nil)
    }

    /// Back from the import to the welcome; an import under way goes on and lands where it is.
    public func back() {
        if case .importing = step {
            step = .start
        }
    }

    /// The chosen file could not be read.
    public func unreadable() {
        step = .importing(isBusy: false, problem: .unreadable)
    }

    /// Imports the key in `privateKey` (zeroed once read), opened with `passphrase`.
    public func importKey(privateKey: inout Data, passphrase: String) async {
        guard privateKey.contains(where: { !Character(UnicodeScalar($0)).isWhitespace }) else {
            privateKey.resetBytes(in: privateKey.startIndex..<privateKey.endIndex)
            step = .importing(isBusy: false, problem: .empty)
            return
        }
        step = .importing(isBusy: true, problem: nil)
        do {
            step = .keyReady(try await keys.importKey(privateKey: &privateKey, passphrase: passphrase))
        } catch let error as KeyError {
            step = .importing(isBusy: false, problem: ImportProblem(error))
        } catch {
            step = .importing(isBusy: false, problem: .notStored(reason: Self.describe(error)))
        }
    }

    /// The public key has been seen: on to the machines.
    public func finish() {
        step = .done
    }

    deinit {
        // nothing to release: the keys are the graph's
    }
}
