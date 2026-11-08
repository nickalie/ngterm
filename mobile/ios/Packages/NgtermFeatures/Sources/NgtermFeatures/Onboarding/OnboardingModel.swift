import Foundation
import NgtermCore
import Observation

/// The first run (mobile/android's `OnboardingViewModel`): the app needs a key before it can reach any machine.
///
/// It starts on the welcome when the app has no key, done when it has one, and on why its keys couldn't be read when
/// they can't be, until a retry reads them. Making or reading a key shows the step busy at once, then waits for the
/// key. An import goes on through Back, and no other key is made or brought until it ends.
@preconcurrency
@MainActor
@Observable
public final class OnboardingModel {
    /// Where the first run is.
    public private(set) var step: OnboardingStep
    /// An import is under way, whichever step shows: it goes on through Back, and the welcome says so.
    public private(set) var isImporting = false
    @ObservationIgnored private let keys: AppKeys

    /// The first run over `keys`.
    public init(keys: AppKeys) {
        self.keys = keys
        step = Self.first(keys)
    }

    // where the first run starts over what `keys` holds now
    private static func first(_ keys: AppKeys) -> OnboardingStep {
        switch keys.readFailure {
        case .unreadable(let reason), .unwritable(let reason):
            .readFailed(reason: reason)

        case nil:
            keys.keys.isEmpty ? .start : .done
        }
    }

    /// Reads the keys again after ``OnboardingStep/readFailed(reason:)``: the welcome when there are none, done when
    /// there are, and the reason again when they still can't be read.
    public func retry() {
        guard case .readFailed = step else {
            return
        }
        keys.readIfUnread()
        step = Self.first(keys)
    }

    /// Makes a new key on the device; nothing happens unless the welcome is up and idle, and no import runs.
    public func create() async {
        guard case .welcome(false, _) = step, !isImporting else {
            return
        }
        step = .welcome(isBusy: true, failure: nil)
        do {
            step = .keyReady(try await keys.generate())
        } catch {
            step = .welcome(isBusy: false, failure: ErrorWords.of(error))
        }
    }

    /// Opens the import from the idle welcome, busy while an earlier import still runs.
    public func startImport() {
        guard case .welcome(false, _) = step else {
            return
        }
        step = .importing(isBusy: isImporting, problem: nil)
    }

    /// Back from the import to the welcome; an import under way goes on: its key shows wherever the user is, and what
    /// was wrong with it only on the import.
    public func back() {
        if case .importing = step {
            step = .start
        }
    }

    /// The chosen file could not be read.
    public func unreadable() {
        step = .importing(isBusy: false, problem: .unreadable)
    }

    /// Imports the key in `privateKey` (zeroed once read, or at once while another import runs, which this one leaves
    /// alone), opened with `passphrase`.
    public func importKey(privateKey: inout Data, passphrase: String) async {
        guard !isImporting else {
            privateKey.resetBytes(in: privateKey.startIndex..<privateKey.endIndex)
            return
        }
        guard privateKey.contains(where: { !Character(UnicodeScalar($0)).isWhitespace }) else {
            privateKey.resetBytes(in: privateKey.startIndex..<privateKey.endIndex)
            step = .importing(isBusy: false, problem: .empty)
            return
        }
        isImporting = true
        step = .importing(isBusy: true, problem: nil)
        let problem: ImportProblem
        do {
            let key = try await keys.importKey(privateKey: &privateKey, passphrase: passphrase)
            isImporting = false
            step = .keyReady(key)
            return
        } catch let error as KeyError {
            problem = ImportProblem(error)
        } catch {
            problem = .notStored(reason: ErrorWords.of(error))
        }
        isImporting = false
        if case .importing = step {
            step = .importing(isBusy: false, problem: problem)
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
