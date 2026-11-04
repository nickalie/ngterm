// swift-tools-version: 6.2

// The mobile core (crates/ngterm-mobile) for the iOS app: its static library and C header as an XCFramework, the
// Swift bindings UniFFI generates over them, the facade the app uses, and the fake core of the tests and of Debug
// builds. scripts/build-ios.sh writes the first two (and the shared test fixtures) into Generated/, which is
// gitignored.

import Foundation
import PackageDescription

let generated = URL(fileURLWithPath: #filePath).deletingLastPathComponent().appendingPathComponent("Generated")
guard
    FileManager.default.fileExists(atPath: generated.appendingPathComponent("NgtermCoreFFI.xcframework").path),
    FileManager.default.fileExists(atPath: generated.appendingPathComponent("NgtermCoreBindings").path)
else {
    fatalError("\(generated.path) is missing or incomplete: run scripts/build-ios.sh (--no-xcode builds the core only)")
}

let strict: [SwiftSetting] = [.treatAllWarnings(as: .error)]

let package = Package(
    name: "NgtermCore",
    platforms: [.iOS(.v26)],
    products: [
        .library(name: "NgtermCore", targets: ["NgtermCore"]),
        .library(name: "NgtermCoreFakes", targets: ["NgtermCoreFakes"]),
    ],
    targets: [
        .binaryTarget(name: "NgtermCoreFFI", path: "Generated/NgtermCoreFFI.xcframework"),
        // the generated bindings: only the facade imports them
        .target(
            name: "NgtermCoreBindings",
            dependencies: ["NgtermCoreFFI"],
            path: "Generated/NgtermCoreBindings",
            swiftSettings: strict
        ),
        .target(name: "NgtermCore", dependencies: ["NgtermCoreBindings"], swiftSettings: strict),
        // compiled into Debug builds only: in any other configuration its sources are empty, so a Release app that
        // links it holds none of it (scripts/build-ios.sh's archive checks)
        .target(
            name: "NgtermCoreFakes",
            dependencies: ["NgtermCore"],
            swiftSettings: strict + [.define("NGTERM_FAKES", .when(configuration: .debug))]
        ),
        .testTarget(name: "NgtermCoreTests", dependencies: ["NgtermCore", "NgtermCoreFakes"], swiftSettings: strict),
    ],
    swiftLanguageModes: [.v6]
)
