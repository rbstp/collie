// swift-tools-version: 6.4

import PackageDescription

let package = Package(
    name: "GhosttyTerminal",
    // macOS only so the snapshot parser tests run with `swift test`.
    platforms: [.iOS("26.0"), .macOS("26.0")],
    products: [
        .library(name: "GhosttyTerminal", targets: ["GhosttyTerminal"])
    ],
    targets: [
        // Built by `just ios-ghostty` (scripts/ghostty/build-xcframework.sh).
        .binaryTarget(name: "GhosttyVt", path: "GhosttyVt.xcframework"),
        .target(name: "GhosttyTerminal", dependencies: ["GhosttyVt"], resources: [.copy("Fonts")]),
        .testTarget(name: "GhosttyTerminalTests", dependencies: ["GhosttyTerminal"]),
    ]
)
