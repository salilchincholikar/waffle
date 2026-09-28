// swift-tools-version:6.2
import Foundation
import PackageDescription

// The Rust engine is built first (`cargo build --release -p waffle-ffi`, or `make app`)
// into the workspace target directory.
let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
let rustLib = root.appendingPathComponent("target/release").path

let package = Package(
    name: "Waffle",
    // Liquid Glass (NSGlassEffectView, glass buttons) needs macOS 26.
    platforms: [.macOS(.v26)],
    targets: [
        .systemLibrary(name: "CWaffle", path: "Sources/CWaffle"),
        // Swift face of the Rust engine (Book, CellPos, CellRect…). Used by the app and the tests.
        .target(
            name: "WaffleBridge",
            dependencies: ["CWaffle"],
            path: "Sources/WaffleBridge",
            linkerSettings: [.unsafeFlags(["-L", rustLib])]
        ),
        .executableTarget(
            name: "Waffle",
            dependencies: ["WaffleBridge"],
            path: "Sources/Waffle",
            swiftSettings: [
                // Env-var test/snapshot hooks (see docs/development.md): debug builds only.
                .define("WAFFLE_DEBUG_HOOKS", .when(configuration: .debug)),
            ],
            linkerSettings: [.linkedFramework("AppKit")]
        ),
        // Plain executable, so it runs without XCTest/Swift Testing (Command Line Tools only).
        .executableTarget(
            name: "BridgeTests",
            dependencies: ["WaffleBridge"],
            path: "Tests/BridgeTests"
        ),
    ],
    swiftLanguageModes: [.v5]
)
