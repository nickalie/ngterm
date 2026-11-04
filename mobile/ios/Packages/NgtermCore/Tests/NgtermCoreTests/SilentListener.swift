import Darwin

/// A TCP listener on the loopback that takes connections and never says a word: an ssh connect to it waits for the
/// server's greeting until something ends it (the connect deadline, a disconnect).
internal final class SilentListener {
    private static let pollMilliseconds = 10
    private static let readSize = 4_096

    /// The port it listens on.
    internal let port: UInt16
    private let listening: Int32
    private var accepted: Int32 = -1

    /// A listener on 127.0.0.1 at a port of the system's choosing; `nil` when the socket can't be had.
    internal init?() {
        let socket = Darwin.socket(AF_INET, SOCK_STREAM, 0)
        guard socket >= 0 else {
            return nil
        }
        var address = sockaddr_in()
        address.sin_family = sa_family_t(AF_INET)
        address.sin_addr.s_addr = inet_addr("127.0.0.1")
        var length = socklen_t(MemoryLayout<sockaddr_in>.size)
        let listened = withUnsafeMutablePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { address in
                bind(socket, address, length) == 0 && listen(socket, SOMAXCONN) == 0
                    && getsockname(socket, address, &length) == 0
            }
        }
        guard listened, fcntl(socket, F_SETFL, O_NONBLOCK) == 0 else {
            close(socket)
            return nil
        }
        listening = socket
        port = UInt16(bigEndian: address.sin_port)
    }

    /// Waits up to `timeout` for a client to connect, and keeps its connection open; whether one did.
    internal func waitForConnection(timeout: Duration) async throws -> Bool {
        let clock = ContinuousClock()
        let deadline = clock.now + timeout
        while clock.now < deadline {
            let connection = accept(listening, nil, nil)
            if connection >= 0 {
                accepted = connection
                return fcntl(connection, F_SETFL, O_NONBLOCK) == 0
            }
            try await Task.sleep(for: .milliseconds(Self.pollMilliseconds))
        }
        return false
    }

    /// Waits up to `timeout` for the client to close its connection, reading what it sends meanwhile (its greeting);
    /// whether it closed.
    internal func waitForClose(timeout: Duration) async throws -> Bool {
        let clock = ContinuousClock()
        let deadline = clock.now + timeout
        var buffer = [UInt8](repeating: 0, count: Self.readSize)
        while clock.now < deadline {
            let read = Darwin.read(accepted, &buffer, buffer.count)
            if read == 0 {
                return true
            }
            if read < 0, errno != EAGAIN {
                // reset rather than closed in good order: closed all the same
                return true
            }
            if read < 0 {
                try await Task.sleep(for: .milliseconds(Self.pollMilliseconds))
            }
        }
        return false
    }

    deinit {
        if accepted >= 0 {
            close(accepted)
        }
        close(listening)
    }
}
