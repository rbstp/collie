import Foundation
import Synchronization

/// collied's control protocol (crates/collied/src/control.rs): one JSON object per line.
enum Request: Equatable, Sendable {
    case pair
    case peersList
    case watch
    case confirm(accept: Bool)

    /// collied refuses unknown fields: these are the exact lines it expects.
    var line: Data {
        let json = switch self {
        case .pair: #"{"cmd":"pair"}"#
        case .peersList: #"{"cmd":"peers_list"}"#
        case .watch: #"{"cmd":"watch"}"#
        case .confirm(let accept): #"{"cmd":"confirm","accept":\#(accept)}"#
        }
        return Data((json + "\n").utf8)
    }
}

struct Candidate: Decodable, Equatable, Sendable {
    let deviceLabel: String
    let nodeName: String
    let stableId: String
    let login: String
    let userId: Int64
    let terminalKey: String?
    let replaces: Bool
    let previousTerminalKey: String?

    init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        deviceLabel = try c.decode(String.self, forKey: .deviceLabel)
        nodeName = try c.decode(String.self, forKey: .nodeName)
        stableId = try c.decode(String.self, forKey: .stableId)
        login = try c.decode(String.self, forKey: .login)
        userId = try c.decode(Int64.self, forKey: .userId)
        terminalKey = try c.decodeIfPresent(String.self, forKey: .terminalKey)
        replaces = try c.decodeIfPresent(Bool.self, forKey: .replaces) ?? false
        previousTerminalKey = try c.decodeIfPresent(String.self, forKey: .previousTerminalKey)
    }

    private enum CodingKeys: String, CodingKey {
        case deviceLabel, nodeName, stableId, login, userId, terminalKey, replaces, previousTerminalKey
    }

    /// Same as `Candidate::terminal_key_change`.
    var terminalKeyChange: String {
        switch (terminalKey, previousTerminalKey) {
        case (nil, _): "none"
        case (_?, nil): "new"
        case let (k?, p?) where k == p: "unchanged"
        default: "replaces the existing one"
        }
    }
}

struct Peer: Decodable, Equatable, Sendable {
    let label: String
    let login: String
    let stableId: String
    let pairedAt: UInt64
}

enum Reply: Equatable, Sendable {
    case invite(uri: String, expiresInSecs: Int)
    case confirm(Candidate)
    case pairDone(paired: Bool, detail: String)
    case peers([Peer])
    case watch(pendingApprovals: Int)
    case error(String)
    case other

    static func decode(_ line: Data) throws -> Reply {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(Wire.self, from: line).reply
    }

    private struct Wire: Decodable {
        let reply: Reply

        private enum Keys: String, CodingKey {
            case type, uri, expiresInSecs, paired, detail, peers, pendingApprovals, message
        }

        init(from decoder: any Decoder) throws {
            let c = try decoder.container(keyedBy: Keys.self)
            reply = switch try c.decode(String.self, forKey: .type) {
            case "invite":
                .invite(
                    uri: try c.decode(String.self, forKey: .uri),
                    expiresInSecs: try c.decode(Int.self, forKey: .expiresInSecs))
            case "confirm": .confirm(try Candidate(from: decoder))
            case "pair_done":
                .pairDone(
                    paired: try c.decode(Bool.self, forKey: .paired),
                    detail: try c.decode(String.self, forKey: .detail))
            case "peers": .peers(try c.decode([Peer].self, forKey: .peers))
            case "watch": .watch(pendingApprovals: try c.decode(Int.self, forKey: .pendingApprovals))
            case "error": .error(try c.decode(String.self, forKey: .message))
            default: .other
            }
        }
    }
}

/// Like `collied::printable` (`char::escape_debug`): control, format (bidi included), separator,
/// combining, default-ignorable and non-ASCII space characters from the phone are shown escaped,
/// never interpreted.
func printable(_ s: String) -> String {
    var out = ""
    for scalar in s.unicodeScalars {
        switch scalar {
        case "\0": out += "\\0"
        case "\t": out += "\\t"
        case "\n": out += "\\n"
        case "\r": out += "\\r"
        default:
            let p = scalar.properties
            let hidden = switch p.generalCategory {
            case .control, .format, .lineSeparator, .paragraphSeparator, .privateUse, .surrogate, .unassigned: true
            case .spaceSeparator: scalar != " "
            default: p.isGraphemeExtend || p.isDefaultIgnorableCodePoint
            }
            if hidden {
                out += "\\u{\(String(scalar.value, radix: 16))}"
            } else {
                out.unicodeScalars.append(scalar)
            }
        }
    }
    return out
}

enum ControlError: Error, Equatable {
    case pathTooLong
    case connect(Int32)
    case io(Int32)
    case lineTooLong
    case closed
}

/// collied's MAX_LINE.
let maxLine = 64 * 1024

struct LineFramer {
    private var buffer: [UInt8] = []

    mutating func push(_ bytes: some Collection<UInt8>) throws -> [Data] {
        buffer.append(contentsOf: bytes)
        var lines: [Data] = []
        while let end = buffer.firstIndex(of: UInt8(ascii: "\n")) {
            guard end <= maxLine else { throw ControlError.lineTooLong }
            lines.append(Data(buffer[..<end]))
            buffer.removeSubrange(...end)
        }
        guard buffer.count <= maxLine else { throw ControlError.lineTooLong }
        return lines
    }
}

/// One connection to the control socket. The socket is 0600 in collied's 0700 data dir, and
/// collied checks the peer's uid: only this user's processes get this far.
final class ControlConnection: Sendable {
    private let fd: Int32
    private let open = Mutex(true)
    let lines: AsyncThrowingStream<Data, any Error>

    static func connect(path: String) throws -> ControlConnection {
        var addr = sockaddr_un()
        let bytes = Array(path.utf8)
        guard bytes.count < MemoryLayout.size(ofValue: addr.sun_path) else { throw ControlError.pathTooLong }
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw ControlError.connect(errno) }
        addr.sun_family = sa_family_t(AF_UNIX)
        addr.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        withUnsafeMutableBytes(of: &addr.sun_path) { $0.copyBytes(from: bytes) }
        let rc = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard rc == 0 else {
            let err = errno
            Darwin.close(fd)
            throw ControlError.connect(err)
        }
        return ControlConnection(fd: fd)
    }

    init(fd: Int32) {
        self.fd = fd
        var on: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
        let (lines, continuation) = AsyncThrowingStream<Data, any Error>.makeStream()
        self.lines = lines
        continuation.onTermination = { [self] _ in close() }
        Thread.detachNewThread { [self] in read(into: continuation) }
    }

    func send(_ request: Request) throws {
        try write(request.line)
    }

    func write(_ data: Data) throws {
        try open.withLock { open in
            guard open else { throw ControlError.closed }
            try data.withUnsafeBytes { buf in
                var offset = 0
                while offset < buf.count {
                    let n = Darwin.write(fd, buf.baseAddress! + offset, buf.count - offset)
                    if n < 0 {
                        if errno == EINTR { continue }
                        throw ControlError.io(errno)
                    }
                    offset += n
                }
            }
        }
    }

    /// Ends the connection: collied sees EOF, which cancels a pairing as Ctrl-C does.
    func close() {
        open.withLock { open in
            if open { _ = shutdown(fd, SHUT_RDWR) }
        }
    }

    private func read(into continuation: AsyncThrowingStream<Data, any Error>.Continuation) {
        var framer = LineFramer()
        var buf = [UInt8](repeating: 0, count: 16 * 1024)
        var failure: (any Error)?
        while true {
            let n = buf.withUnsafeMutableBytes { Darwin.read(fd, $0.baseAddress, $0.count) }
            if n == 0 { break }
            if n < 0 {
                if errno == EINTR { continue }
                failure = ControlError.io(errno)
                break
            }
            do {
                for line in try framer.push(buf[..<n]) { continuation.yield(line) }
            } catch {
                failure = error
                break
            }
        }
        open.withLock { open in
            open = false
            Darwin.close(fd)
        }
        continuation.finish(throwing: failure)
    }
}

/// One request, its first reply line.
func request(_ request: Request, socket: String) async throws -> Reply {
    let conn = try ControlConnection.connect(path: socket)
    defer { conn.close() }
    try conn.send(request)
    for try await line in conn.lines {
        return try Reply.decode(line)
    }
    throw ControlError.closed
}

enum DaemonState: Equatable, Sendable {
    case off
    case running(pendingApprovals: Int)
    /// A collied from before the watch command: running, but without the count.
    case outdated
}

/// The menu bar status, pushed by collied over one held connection. The only polling is a
/// failing connect every `retry` while collied is off.
func watchDaemon(socket: String, retry: Duration, update: @Sendable (DaemonState) async -> Void) async {
    while !Task.isCancelled {
        var outdated = false
        if let conn = try? ControlConnection.connect(path: socket) {
            do {
                try conn.send(.watch)
                for try await line in conn.lines {
                    switch try Reply.decode(line) {
                    case .watch(let n): await update(.running(pendingApprovals: n))
                    case .error:
                        outdated = true
                        await update(.outdated)
                    default: break
                    }
                }
            } catch {}
            conn.close()
        }
        if !outdated { await update(.off) }
        try? await Task.sleep(for: retry)
    }
}
