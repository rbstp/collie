import Foundation

/// Display choices kept on this device only, in the state dir rather than UserDefaults.
struct DevicePrefs: Codable, Equatable {
    var wrapLines = true
    var keepKeyboard = false

    static let file: URL? = try? StateDirectory.prepare().appending(path: "prefs.json")

    static func load(from file: URL?) -> DevicePrefs {
        file.flatMap { try? Data(contentsOf: $0) }.flatMap { try? JSONDecoder().decode(Self.self, from: $0) } ?? DevicePrefs()
    }

    func save(to file: URL?) {
        guard let file, let data = try? JSONEncoder().encode(self) else { return }
        try? data.write(to: file, options: .atomic)
    }
}

extension DevicePrefs {
    /// A file saved before a field existed keeps the fields it has.
    init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let defaults = DevicePrefs()
        wrapLines = try container.decodeIfPresent(Bool.self, forKey: .wrapLines) ?? defaults.wrapLines
        keepKeyboard = try container.decodeIfPresent(Bool.self, forKey: .keepKeyboard) ?? defaults.keepKeyboard
    }
}
