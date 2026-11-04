// swift-tools-version: 6.2

// The iOS app's design system: tokens, the status glyphs, the terminal palettes, the fonts and the components.
// scripts/build-ios.sh copies the fonts shared with Android (and their licenses) into Generated/, which is gitignored.

import Foundation
import PackageDescription

let fonts = URL(fileURLWithPath: #filePath).deletingLastPathComponent().appendingPathComponent("Generated/Fonts")
guard FileManager.default.fileExists(atPath: fonts.path) else {
    fatalError("\(fonts.path) is missing: run scripts/build-ios.sh (--no-xcode copies it)")
}

let strict: [SwiftSetting] = [.treatAllWarnings(as: .error)]

let package = Package(
    name: "NgtermDesign",
    defaultLocalization: "en",
    platforms: [.iOS(.v26)],
    products: [.library(name: "NgtermDesign", targets: ["NgtermDesign"])],
    dependencies: [
        .package(url: "https://github.com/pointfreeco/swift-snapshot-testing", exact: "1.19.7")
    ],
    targets: [
        // the fonts live outside the target (Generated/ is the package's one gitignored directory), so the resource
        // path climbs out of Sources/NgtermDesign
        .target(name: "NgtermDesign", resources: [.copy("../../Generated/Fonts")], swiftSettings: strict),
        .testTarget(
            name: "NgtermDesignTests",
            dependencies: [
                "NgtermDesign",
                .product(name: "SnapshotTesting", package: "swift-snapshot-testing"),
            ],
            exclude: ["__Snapshots__"],
            swiftSettings: strict
        ),
    ],
    swiftLanguageModes: [.v6]
)
