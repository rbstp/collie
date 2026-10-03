#if DEBUG && canImport(UIKit)
import Foundation
import SwiftUI

/// `--terminal-demo <file>`: shows the first "text" string of a herdr `pane.read` JSON
/// response (or the raw file) in the terminal view. Debug builds only.
public struct GhosttyTerminalDemo: View {
    let snapshot: String

    public init?(arguments: [String]) {
        guard let flag = arguments.firstIndex(of: "--terminal-demo"), flag + 1 < arguments.count,
            let data = FileManager.default.contents(atPath: arguments[flag + 1])
        else { return nil }
        let json = try? JSONSerialization.jsonObject(with: data)
        snapshot = json.flatMap(Self.firstText) ?? String(decoding: data, as: UTF8.self)
    }

    public var body: some View {
        GhosttyTerminalView(snapshot: snapshot)
            .ignoresSafeArea(edges: .bottom)
            .background(Color(red: 0x28 / 255, green: 0x2C / 255, blue: 0x34 / 255))
    }

    private static func firstText(_ value: Any) -> String? {
        if let object = value as? [String: Any] {
            if let text = object["text"] as? String { return text }
            return object.keys.sorted().lazy.compactMap { object[$0].flatMap(firstText) }.first
        }
        if let array = value as? [Any] {
            return array.lazy.compactMap(firstText).first
        }
        return nil
    }
}
#endif
