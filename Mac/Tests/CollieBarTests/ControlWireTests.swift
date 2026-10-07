import Foundation
import Testing

struct ControlWireTests {
    @Test func requestsAreTheLinesColliedExpects() {
        let lines = [Request.pair, .peersList, .watch, .confirm(accept: true), .confirm(accept: false)]
            .map { String(decoding: $0.line, as: UTF8.self) }
        #expect(
            lines == [
                "{\"cmd\":\"pair\"}\n", "{\"cmd\":\"peers_list\"}\n", "{\"cmd\":\"watch\"}\n",
                "{\"cmd\":\"confirm\",\"accept\":true}\n", "{\"cmd\":\"confirm\",\"accept\":false}\n",
            ])
    }

    private func decode(_ json: String) throws -> Reply {
        try Reply.decode(Data(json.utf8))
    }

    @Test func repliesDecode() throws {
        #expect(
            try decode(#"{"type":"invite","uri":"collie://pair#v=1","expires_in_secs":120}"#)
                == .invite(uri: "collie://pair#v=1", expiresInSecs: 120))
        #expect(
            try decode(#"{"type":"pair_done","paired":false,"detail":"not confirmed on the machine"}"#)
                == .pairDone(paired: false, detail: "not confirmed on the machine"))
        #expect(try decode(#"{"type":"watch","pending_approvals":2}"#) == .watch(pendingApprovals: 2))
        #expect(
            try decode(#"{"type":"error","message":"a pairing window is already open"}"#)
                == .error("a pairing window is already open"))
        #expect(try decode(#"{"type":"noted"}"#) == .other)
        let peers = try decode(
            #"{"type":"peers","owner_user_id":7,"peers":[{"stable_id":"nP","user_id":7,"login":"me@example.com","#
                + #""label":"iPhone","paired_at":1700000000000,"tls_key":"k","terminal_key":null}]}"#)
        #expect(peers == .peers([Peer(label: "iPhone", login: "me@example.com", stableId: "nP", pairedAt: 1_700_000_000_000)]))
        #expect(try decode(#"{"type":"peers","owner_user_id":null,"peers":[]}"#) == .peers([]))
    }

    @Test func candidateDecodesWithAndWithoutOptionalKeys() throws {
        let base = #""device_label":"iPhone","node_name":"phone","stable_id":"nP","login":"me","user_id":7,"tls_key":"k""#
        guard case .confirm(let old) = try decode("{\"type\":\"confirm\",\(base)}") else {
            Issue.record("not a confirm")
            return
        }
        #expect(old.deviceLabel == "iPhone" && old.nodeName == "phone" && old.stableId == "nP")
        #expect(old.login == "me" && old.userId == 7)
        #expect(old.terminalKey == nil && !old.replaces && old.previousTerminalKey == nil)
        guard
            case .confirm(let full) = try decode(
                "{\"type\":\"confirm\",\(base),\"terminal_key\":\"t1\",\"replaces\":true,\"previous_terminal_key\":\"t0\"}")
        else {
            Issue.record("not a confirm")
            return
        }
        #expect(full.terminalKey == "t1" && full.replaces && full.previousTerminalKey == "t0")
    }

    @Test func terminalKeyChangeMatchesCollied() throws {
        func change(_ key: String?, _ previous: String?) throws -> String {
            var json = #"{"type":"confirm","device_label":"d","node_name":"n","stable_id":"s","login":"l","user_id":1"#
            if let key { json += ",\"terminal_key\":\"\(key)\"" }
            if let previous { json += ",\"previous_terminal_key\":\"\(previous)\"" }
            guard case .confirm(let c) = try decode(json + "}") else { return "" }
            return c.terminalKeyChange
        }
        #expect(try change(nil, nil) == "none")
        #expect(try change(nil, "a") == "none")
        #expect(try change("a", nil) == "new")
        #expect(try change("a", "a") == "unchanged")
        #expect(try change("a", "b") == "replaces the existing one")
    }

    @Test func printableEscapesWhatThePhoneCouldHide() {
        #expect(printable("Rich\u{2019}s \"iPhone\" \\ 15") == "Rich\u{2019}s \"iPhone\" \\ 15")
        #expect(printable("ab\u{202e}cd") == "ab\\u{202e}cd")
        #expect(printable("a\u{200b}b\u{2066}") == "a\\u{200b}b\\u{2066}")
        #expect(printable("a\nb\tc\r") == "a\\nb\\tc\\r")
        #expect(printable("\u{1b}[31mred") == "\\u{1b}[31mred")
        #expect(printable("x\u{7f}\u{2028}") == "x\\u{7f}\\u{2028}")
        #expect(printable("a\u{a0}b\u{301}c\u{3000}d\u{fe0f}e\u{3164}") == "a\\u{a0}b\\u{301}c\\u{3000}d\\u{fe0f}e\\u{3164}")
    }

    @Test func linesAreFramedAcrossReads() throws {
        var framer = LineFramer()
        #expect(try framer.push(Array("{\"a\":".utf8)).isEmpty)
        let lines = try framer.push(Array("1}\n{\"b\":2}\n{\"c\"".utf8))
        #expect(lines.map { String(decoding: $0, as: UTF8.self) } == ["{\"a\":1}", "{\"b\":2}"])
        #expect(try framer.push(Array(":3}\n".utf8)).map { String(decoding: $0, as: UTF8.self) } == ["{\"c\":3}"])
    }

    @Test func overlongLinesAreRefused() throws {
        var framer = LineFramer()
        #expect(try framer.push([UInt8](repeating: 0x61, count: maxLine)).isEmpty)
        #expect(throws: ControlError.lineTooLong) { try framer.push([0x61]) }
        var complete = LineFramer()
        #expect(throws: ControlError.lineTooLong) { try complete.push([UInt8](repeating: 0x61, count: maxLine + 1) + [0x0a]) }
        var fits = LineFramer()
        #expect(try fits.push([UInt8](repeating: 0x61, count: maxLine) + [0x0a]).count == 1)
    }
}
