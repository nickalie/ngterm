// swift-tools-version: 6.2

// The iOS app's features: the app graph, the event routers over the core, and the screens (onboarding, machines,
// sessions, the session, asks and settings).

import PackageDescription

let strict: [SwiftSetting] = [.treatAllWarnings(as: .error)]

let package = Package(
    name: "NgtermFeatures",
    platforms: [.iOS(.v26)],
    products: [.library(name: "NgtermFeatures", targets: ["NgtermFeatures"])],
    dependencies: [
        .package(path: "../NgtermCore"),
        .package(path: "../NgtermDesign"),
        .package(path: "../NgtermTerminal"),
        .package(path: "../NgtermPlatform"),
        .package(url: "https://github.com/pointfreeco/swift-snapshot-testing", exact: "1.19.7"),
    ],
    targets: [
        .target(
            name: "NgtermFeatures",
            dependencies: [
                .product(name: "NgtermCore", package: "NgtermCore"),
                .product(name: "NgtermDesign", package: "NgtermDesign"),
                .product(name: "NgtermTerminal", package: "NgtermTerminal"),
                .product(name: "NgtermPlatform", package: "NgtermPlatform"),
            ],
            swiftSettings: strict
        ),
        .testTarget(
            name: "NgtermFeaturesTests",
            dependencies: [
                "NgtermFeatures",
                .product(name: "NgtermDesign", package: "NgtermDesign"),
                .product(name: "SnapshotTesting", package: "swift-snapshot-testing"),
            ],
            exclude: ["__Snapshots__"],
            swiftSettings: strict
        ),
    ],
    swiftLanguageModes: [.v6]
)
