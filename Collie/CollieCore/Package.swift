// swift-tools-version: 6.4

import PackageDescription

let package = Package(
    name: "CollieCore",
    platforms: [.iOS("26.0")],
    products: [
        .library(name: "CollieCore", targets: ["CollieCore"])
    ],
    targets: [
        .binaryTarget(name: "CollieCoreFFI", path: "CollieCore.xcframework"),
        .target(
            name: "CollieCore",
            dependencies: ["CollieCoreFFI"],
            // Required by the Go runtime and libtailscale linked into the static library.
            linkerSettings: [
                .linkedFramework("CoreFoundation"),
                .linkedFramework("Security"),
                .linkedLibrary("resolv"),
            ]
        ),
    ]
)
