// swift-tools-version: 6.2

// The iOS app's design system: tokens, the status glyphs, the terminal palettes, the fonts and the components.
// scripts/build-ios.sh copies the fonts shared with Android into Generated/, which is gitignored.

import PackageDescription

let strict: [SwiftSetting] = [.treatAllWarnings(as: .error)]

let package = Package(
    name: "NgtermDesign",
    platforms: [.iOS(.v26)],
    products: [.library(name: "NgtermDesign", targets: ["NgtermDesign"])],
    dependencies: [
        .package(url: "https://github.com/pointfreeco/swift-snapshot-testing", exact: "1.19.7")
    ],
    targets: [
        .target(name: "NgtermDesign", swiftSettings: strict),
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
