import Foundation
import Testing

@Test func privacyManifestsDeclareTheUserDefaultsEachBundleReads() throws {
    let app = Bundle.main.bundleURL
    #expect(try userDefaultsReasons(in: app) == ["1C8F.1", "CA92.1"])
    #expect(try userDefaultsReasons(in: app.appending(path: "PlugIns/CollieWidgets.appex")) == ["1C8F.1"])
    #expect(try userDefaultsReasons(in: app.appending(path: "Watch/CollieWatch.app")) == ["CA92.1"])
}

private func userDefaultsReasons(in bundle: URL) throws -> [String] {
    let data = try Data(contentsOf: bundle.appending(path: "PrivacyInfo.xcprivacy"))
    let manifest = try PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any]
    let types = manifest?["NSPrivacyAccessedAPITypes"] as? [[String: Any]] ?? []
    let defaults = types.first { $0["NSPrivacyAccessedAPIType"] as? String == "NSPrivacyAccessedAPICategoryUserDefaults" }
    return (defaults?["NSPrivacyAccessedAPITypeReasons"] as? [String] ?? []).sorted()
}
