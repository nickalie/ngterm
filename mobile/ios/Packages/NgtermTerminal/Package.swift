// swift-tools-version: 6.2

// The terminal view of the iOS app: the row buffer's reader, the Core Text painter, the UIKit view, the key map, the
// text input and the accessory bar.

import PackageDescription

let strict: [SwiftSetting] = [.treatAllWarnings(as: .error)]

let package = Package(
    name: "NgtermTerminal",
    platforms: [.iOS(.v26)],
    products: [.library(name: "NgtermTerminal", targets: ["NgtermTerminal"])],
    dependencies: [
        .package(path: "../NgtermCore"),
        .package(path: "../NgtermDesign"),
        .package(url: "https://github.com/pointfreeco/swift-snapshot-testing", exact: "1.19.7"),
    ],
    targets: [
        .target(
            name: "NgtermTerminal",
            dependencies: [
                .product(name: "NgtermCore", package: "NgtermCore"),
                .product(name: "NgtermDesign", package: "NgtermDesign"),
            ],
            swiftSettings: strict
        ),
        .testTarget(
            name: "NgtermTerminalTests",
            dependencies: [
                "NgtermTerminal",
                .product(name: "NgtermCore", package: "NgtermCore"),
                .product(name: "NgtermCoreFakes", package: "NgtermCore"),
                .product(name: "NgtermDesign", package: "NgtermDesign"),
                .product(name: "SnapshotTesting", package: "swift-snapshot-testing"),
            ],
            exclude: ["__Snapshots__"],
            swiftSettings: strict
        ),
    ],
    swiftLanguageModes: [.v6]
)
