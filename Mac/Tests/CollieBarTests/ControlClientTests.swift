import Foundation
import Synchronization
import Testing

/// A scripted stand-in for collied's control socket, in a temporary directory: these tests
/// never touch the real socket.
final class FakeServer: Sendable {
    let path: String
    private let fd: Int32
    private let open = Mutex(true)

    init() throws {
        path = "/tmp/colliebar-\(UUID().uuidString.prefix(8)).sock"
        fd = socket(AF_UNIX, SOCK_STREAM, 0)
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        addr.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        withUnsafeMutableBytes(of: &addr.sun_path) { $0.copyBytes(from: Array(path.utf8)) }
        let rc = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard rc == 0, listen(fd, 4) == 0 else { throw ControlError.connect(errno) }
    }

    func accept() async -> Remote {
        let fd = self.fd
        return await withCheckedContinuation { continuation in
            Thread.detachNewThread {
                continuation.resume(returning: Remote(ControlConnection(fd: Darwin.accept(fd, nil, nil))))
            }
        }
    }

    func stop() {
        open.withLock { open in
            guard open else { return }
            open = false
            Darwin.close(fd)
            unlink(path)
        }
    }

    /// The daemon's end of one connection. Lines are collected by one long-lived reader, so
    /// a timed-out wait does not cancel the stream (which would close the connection).
    final class Remote: Sendable {
        let conn: ControlConnection
        private let received = Mutex<(lines: [String], eof: Bool)>(([], false))

        init(_ conn: ControlConnection) {
            self.conn = conn
            Task { [self] in
                do {
                    for try await line in conn.lines {
                        received.withLock { $0.lines.append(String(decoding: line, as: UTF8.self)) }
                    }
                } catch {}
                received.withLock { $0.eof = true }
            }
        }

        /// The next line, or nil on EOF or after `timeout`.
        func next(timeout: Duration = .seconds(2)) async -> String? {
            let deadline = ContinuousClock.now + timeout
            while ContinuousClock.now < deadline {
                let (line, eof) = received.withLock { r -> (String?, Bool) in
                    r.lines.isEmpty ? (nil, r.eof) : (r.lines.removeFirst(), false)
                }
                if let line { return line }
                if eof { return nil }
                try? await Task.sleep(for: .milliseconds(10))
            }
            return nil
        }

        var eof: Bool { received.withLock { $0.eof } }

        func reply(_ json: String) throws {
            try conn.write(Data((json + "\n").utf8))
        }

        func close() { conn.close() }
    }
}

struct ControlClientTests {
    @Test func confirmIsSentOnlyWhenAnswered() async throws {
        let server = try FakeServer()
        defer { server.stop() }
        let client = try ControlConnection.connect(path: server.path)
        let daemon = await server.accept()
        var lines = client.lines.makeAsyncIterator()

        try client.send(.pair)
        #expect(await daemon.next() == #"{"cmd":"pair"}"#)
        try daemon.reply(#"{"type":"invite","uri":"collie://pair#v=1","expires_in_secs":120}"#)
        try daemon.reply(
            #"{"type":"confirm","device_label":"d","node_name":"n","stable_id":"s","login":"l","user_id":1,"tls_key":"k"}"#)
        #expect(try await lines.next().map { try Reply.decode($0) } == .invite(uri: "collie://pair#v=1", expiresInSecs: 120))
        guard case .confirm = try await lines.next().map({ try Reply.decode($0) }) else {
            Issue.record("no confirm")
            return
        }
        #expect(await daemon.next(timeout: .milliseconds(300)) == nil, "nothing is sent unasked")

        try client.send(.confirm(accept: true))
        #expect(await daemon.next() == #"{"cmd":"confirm","accept":true}"#)
        try daemon.reply(#"{"type":"pair_done","paired":true,"detail":"paired d (s)"}"#)
        #expect(try await lines.next().map { try Reply.decode($0) } == .pairDone(paired: true, detail: "paired d (s)"))
    }

    @Test func closingIsEOFForCollied() async throws {
        let server = try FakeServer()
        defer { server.stop() }
        let client = try ControlConnection.connect(path: server.path)
        let daemon = await server.accept()
        try client.send(.pair)
        #expect(await daemon.next() == #"{"cmd":"pair"}"#)
        client.close()
        #expect(await daemon.next() == nil)
        #expect(daemon.eof)
        #expect(throws: ControlError.closed) { try client.send(.confirm(accept: true)) }
    }

    @Test func noDaemonIsAConnectError() throws {
        #expect(throws: ControlError.connect(ENOENT)) {
            try ControlConnection.connect(path: "/tmp/colliebar-missing-\(UUID().uuidString.prefix(8)).sock")
        }
        #expect(throws: ControlError.pathTooLong) {
            try ControlConnection.connect(path: "/tmp/" + String(repeating: "x", count: 120))
        }
    }

    @Test func watchFollowsTheDaemon() async throws {
        let server = try FakeServer()
        defer { server.stop() }
        let (states, sink) = AsyncStream<DaemonState>.makeStream()
        let watcher = Task {
            await watchDaemon(socket: server.path, retry: .milliseconds(20)) { sink.yield($0) }
        }
        defer { watcher.cancel() }
        var seen = states.makeAsyncIterator()

        let daemon = await server.accept()
        #expect(await daemon.next() == #"{"cmd":"watch"}"#)
        try daemon.reply(#"{"type":"watch","pending_approvals":0}"#)
        #expect(await seen.next() == .running(pendingApprovals: 0))
        try daemon.reply(#"{"type":"watch","pending_approvals":1}"#)
        #expect(await seen.next() == .running(pendingApprovals: 1))
        daemon.close()
        #expect(await seen.next() == .off, "EOF means collied stopped")

        let old = await server.accept()
        #expect(await old.next() == #"{"cmd":"watch"}"#)
        try old.reply(#"{"type":"error","message":"unknown variant `watch`"}"#)
        old.close()
        #expect(await seen.next() == .outdated, "an older collied still runs")
        let again = await server.accept()
        #expect(await again.next() == #"{"cmd":"watch"}"#)
        server.stop()
        again.close()
        #expect(await seen.next() == .off)
    }
}
