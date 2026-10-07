import CoreImage
import CoreImage.CIFilterBuiltins
import SwiftUI

/// `collied pair` in a window: the QR, then the candidate phone to accept or refuse on this
/// Mac. The invite is never shown as text, copied, logged or kept: only its QR image lives
/// in this view's state, and closing the window closes the connection, which cancels.
struct PairingView: View {
    let daemon: Daemon
    @Environment(\.dismissWindow) private var dismissWindow
    @State private var phase: Phase = .connecting
    @State private var connection: ControlConnection?
    @State private var armed = false
    @State private var answered = false

    enum Phase {
        case connecting
        case invite(CGImage, until: Date)
        case confirm(Candidate, until: Date)
        case finished(String)
    }

    var body: some View {
        VStack(spacing: 16) {
            switch phase {
            case .connecting:
                ProgressView()
            case .invite(let qr, let until):
                Text("Scan with the Collie app")
                    .font(.headline)
                Image(decorative: qr, scale: 1)
                    .interpolation(.none)
                    .resizable()
                    .frame(width: 240, height: 240)
                    .padding(16)
                    .background(.white)
                countdown(until) { "Waiting up to \($0) s for the phone" }
                Button("Cancel", role: .cancel) { dismissWindow(id: "pair") }
                    .keyboardShortcut(.cancelAction)
            case .confirm(let c, let until):
                candidate(c)
                TimelineView(.periodic(from: .now, by: 1)) { context in
                    let left = max(0, Int(until.timeIntervalSince(context.date).rounded(.up)))
                    VStack(spacing: 12) {
                        Text(verbatim: "Pair this phone? (\(left) s)")
                        HStack {
                            Button("Don't Pair") { answer(false) }
                                .keyboardShortcut(.cancelAction)
                                .disabled(answered)
                            // No default button, and live only after a second: Return or a
                            // click meant for the QR window cannot approve.
                            Button("Pair") { answer(true) }
                                .disabled(answered || !armed || left == 0)
                        }
                    }
                }
            case .finished(let message):
                Text(verbatim: message)
                    .multilineTextAlignment(.center)
                Button("OK") { dismissWindow(id: "pair") }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(24)
        .frame(minWidth: 360)
        .task { await run() }
    }

    private func candidate(_ c: Candidate) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("A phone presented the pairing code:")
                .font(.headline)
            Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 4) {
                row("device", printable(c.deviceLabel))
                row("node", "\(printable(c.nodeName)) (\(printable(c.stableId)))")
                row("user", "\(printable(c.login)) (\(c.userId))")
                row("terminal key", c.terminalKeyChange)
            }
            if c.replaces {
                Text("replaces an existing pairing of this node")
            }
        }
        .textSelection(.disabled)
    }

    private func row(_ name: String, _ value: String) -> some View {
        GridRow {
            Text(verbatim: name).foregroundStyle(.secondary)
            Text(verbatim: value)
        }
    }

    private func countdown(_ until: Date, _ text: @escaping (Int) -> String) -> some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            Text(verbatim: text(max(0, Int(until.timeIntervalSince(context.date).rounded(.up)))))
                .foregroundStyle(.secondary)
        }
    }

    private func answer(_ accept: Bool) {
        guard !answered else { return }
        answered = true
        do {
            try connection?.send(.confirm(accept: accept))
        } catch {
            phase = .finished("collied closed the pairing")
        }
    }

    private func run() async {
        let conn: ControlConnection
        do {
            conn = try ControlConnection.connect(path: daemon.socketPath)
        } catch {
            phase = .finished("collied is not running")
            return
        }
        connection = conn
        defer {
            conn.close()
            connection = nil
        }
        do {
            try conn.send(.pair)
            for try await line in conn.lines {
                switch try Reply.decode(line) {
                case .invite(let uri, let secs):
                    guard let qr = qrImage(uri) else {
                        phase = .finished("cannot draw the pairing QR")
                        return
                    }
                    phase = .invite(qr, until: .now + TimeInterval(secs))
                case .confirm(let c):
                    armed = false
                    answered = false
                    phase = .confirm(c, until: .now + 60)
                    Task {
                        try? await Task.sleep(for: .seconds(1))
                        armed = true
                    }
                case .pairDone(_, let detail):
                    phase = .finished(printable(detail))
                    daemon.refreshPeers()
                    return
                case .error(let message):
                    phase = .finished(printable(message))
                    return
                default:
                    phase = .finished("unexpected reply from collied")
                    return
                }
            }
            if !Task.isCancelled { phase = .finished("collied closed the pairing") }
        } catch {
            if !Task.isCancelled { phase = .finished("the pairing connection failed") }
        }
    }

    private func qrImage(_ text: String) -> CGImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage else { return nil }
        let scaled = output.transformed(by: CGAffineTransform(scaleX: 8, y: 8))
        return CIContext().createCGImage(scaled, from: scaled.extent)
    }
}
