import Foundation
import NgtermCore

/// A pane's chat's uploads (mobile/android's `ChatUploads`), one at a time.
///
/// The file's bytes are read off the main actor, sent to the pane's machine through the core, and the stored path
/// handed on (the composer puts it into the prompt at the cursor). ``cancel()`` stops the one on its way while bytes of
/// it are still to go, and the machine keeps nothing of it. Once every byte has gone (the upload is saving) the machine
/// stores the file whatever the device does, so a cancel holds off and the upload runs to its path.
@preconcurrency
@MainActor
internal final class ChatUploads {
    /// One upload as it goes: the machine, the file's name, how to read its bytes, and its token.
    private struct Request: Sendable {
        let machineId: String
        let name: String
        let read: @Sendable () async throws -> Data
        let token: UploadCancel
    }

    private let machineId: String
    private let core: any MobileCoreClient
    private let show: @MainActor (ChatUpload?) -> Void
    private let current: @MainActor () -> ChatUpload?
    private let stored: @MainActor (String) -> Void
    // the upload on its way: its token, and its number, so an answer to one cancelled or replaced is dropped
    private var token: UploadCancel?
    private var number = 0
    private var task: Task<Void, Never>?

    /// The uploads to `machineId` through `core`, shown through `show` (what shows now is `current`), each stored path
    /// handed to `stored`.
    internal init(
        machineId: String,
        core: any MobileCoreClient,
        show: @escaping @MainActor (ChatUpload?) -> Void,
        current: @escaping @MainActor () -> ChatUpload?,
        stored: @escaping @MainActor (String) -> Void
    ) {
        self.machineId = machineId
        self.core = core
        self.show = show
        self.current = current
        self.stored = stored
    }

    private static func send(
        _ request: Request,
        core: any MobileCoreClient,
        progress: @escaping @Sendable (UInt64, UInt64) -> Void
    ) async -> Result<String, UploadProblem> {
        let bytes: Data
        do {
            bytes = try await Task.detached(operation: request.read).value
        } catch let problem as UploadProblem {
            // a reader that measured the file first says it is too large
            return .failure(problem)
        } catch {
            return .failure(.unreadable)
        }
        guard bytes.count <= ChatUpload.cap else {
            return .failure(.tooLarge)
        }
        do {
            let path = try await core.uploadFile(
                machineId: request.machineId,
                name: request.name,
                bytes: bytes,
                progress: progress,
                cancel: request.token
            )
            return .success(path)
        } catch let error as MobileError {
            return .failure(.refused(reason: error.reason))
        } catch {
            return .failure(.refused(reason: MobileError.Cancelled.reason))
        }
    }

    /// Sends the file `name` of `size` bytes (`nil`: unknown) that `read` gives, unless one is on its way.
    internal func attach(name: String, size: Int?, read: @escaping @Sendable () async throws -> Data) {
        guard current()?.isSending != true else {
            return
        }
        if let size, size > ChatUpload.cap {
            show(.failed(name: name, problem: .tooLarge))
            return
        }
        number += 1
        let mine = number
        let cancel = UploadCancel()
        token = cancel
        show(.sending(name: name, sent: 0, total: UInt64(size ?? 0), isSaving: false))
        let request = Request(machineId: machineId, name: name, read: read, token: cancel)
        task = Task { [weak self, core] in
            let outcome = await Self.send(request, core: core) { sent, total in
                Task { @MainActor in
                    self?.progressed(mine, sent: sent, total: total)
                }
            }
            self?.ended(mine, name: name, outcome)
        }
    }

    /// Stops the upload on its way, unless every byte has gone.
    internal func cancel() {
        guard case .sending(_, _, _, let isSaving) = current(), !isSaving else {
            return
        }
        stop()
        show(nil)
    }

    /// Stops the upload on its way, if any, whatever it has sent: the chat closes.
    internal func stop() {
        token?.cancel()
        token = nil
        number += 1
        task?.cancel()
    }

    private func progressed(_ mine: Int, sent: UInt64, total: UInt64) {
        guard mine == number, case .sending(let name, let before, _, _) = current(), sent >= before else {
            return
        }
        show(.sending(name: name, sent: sent, total: total, isSaving: sent >= total))
    }

    private func ended(_ mine: Int, name: String, _ outcome: Result<String, UploadProblem>) {
        // a cancelled upload, or one another replaced, says nothing more
        guard mine == number else {
            return
        }
        token = nil
        switch outcome {
        case .success(let path):
            show(nil)
            stored(path)

        case .failure(let problem):
            show(.failed(name: name, problem: problem))
        }
    }

    isolated deinit {
        task?.cancel()
    }
}
