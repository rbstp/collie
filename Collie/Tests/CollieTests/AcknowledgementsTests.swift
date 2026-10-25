import Testing

@testable import Collie

@Test func bundledAcknowledgementsListLibtailscaleWithItsLicense() throws {
    let items = Acknowledgement.bundled
    let libtailscale = try #require(items.first { $0.name == "libtailscale" })
    #expect(libtailscale.license == "BSD-3-Clause")
    #expect(libtailscale.text.contains("Tailscale & AUTHORS"))
    #expect(items.contains { $0.name == "libghostty-vt" })
    #expect(items.contains { $0.name == "tokio" })
}
