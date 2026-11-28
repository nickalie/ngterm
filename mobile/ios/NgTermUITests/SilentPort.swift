import Foundation

/// A port on the Mac's 127.0.0.1 that takes each TCP connection and never says a word: an ssh server that never
/// answers, for a test of how long a connect waits before it gives up.
///
/// It keeps, for each connection, when it was taken and when the other side closed it, on the system's uptime clock.
internal final class SilentPort: @unchecked Sendable {
    /// One connection: when it was taken, and when the other side closed it (nil while it is open).
    internal struct Call: Equatable {
        internal let taken: TimeInterval
        internal let closed: TimeInterval?

        /// How long it was open, once closed.
        internal var seconds: TimeInterval? {
            closed.map { $0 - taken }
        }
    }

    private static let backlog: Int32 = 8
    private static let readSize = 512

    /// The port it listens on.
    internal let port: UInt16
    private let listener: Int32
    private let lock = NSLock()
    private var taken: [Call] = []

    /// The connections so far, in the order they were taken.
    internal var calls: [Call] {
        lock.withLock { taken }
    }

    /// Listens on a free port of 127.0.0.1.
    ///
    /// - Throws: `POSIXError` when the port can't be opened.
    internal init() throws {
        let socketNumber = socket(AF_INET, SOCK_STREAM, 0)
        guard socketNumber >= 0 else {
            throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
        }
        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_port = 0
        address.sin_addr.s_addr = inet_addr("127.0.0.1")
        var length = socklen_t(MemoryLayout<sockaddr_in>.size)
        let isOpen = withUnsafeMutablePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { generic in
                bind(socketNumber, generic, length) == 0 && listen(socketNumber, Self.backlog) == 0
                    && getsockname(socketNumber, generic, &length) == 0
            }
        }
        guard isOpen else {
            let code = POSIXErrorCode(rawValue: errno) ?? .EIO
            close(socketNumber)
            throw POSIXError(code)
        }
        listener = socketNumber
        port = UInt16(bigEndian: address.sin_port)
        Thread { [self] in takeConnections() }.start()
    }

    /// Stops listening; the connections still open stay open until their other side closes them.
    internal func stop() {
        shutdown(listener, SHUT_RDWR)
        close(listener)
    }

    private func takeConnections() {
        while true {
            let connection = accept(listener, nil, nil)
            guard connection >= 0 else {
                return
            }
            let index = lock.withLock {
                taken.append(Call(taken: ProcessInfo.processInfo.systemUptime, closed: nil))
                return taken.count - 1
            }
            Thread { [self] in readUntilClosed(connection, index: index) }.start()
        }
    }

    // reads what the other side says (an ssh client's version line) and drops it, until it closes the connection
    private func readUntilClosed(_ connection: Int32, index: Int) {
        var buffer = [UInt8](repeating: 0, count: Self.readSize)
        while recv(connection, &buffer, buffer.count, 0) > 0 {
            continue
        }
        let closed = ProcessInfo.processInfo.systemUptime
        close(connection)
        lock.withLock {
            taken[index] = Call(taken: taken[index].taken, closed: closed)
        }
    }

    deinit {
        // stop() closes the listener; a test calls it in its teardown
    }
}
