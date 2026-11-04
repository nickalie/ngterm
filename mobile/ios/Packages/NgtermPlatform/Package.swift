// swift-tools-version: 6.2

// The iOS app's platform services: the Keychain key vault and host keys, the JSON stores, the network monitor,
// notifications, the lifecycle and background refresh. The Keychain's own tests are the app's hosted ones
// (NgTermTests): an unhosted test bundle has no keychain.

import PackageDescription

let strict: [SwiftSetting] = [.treatAllWarnings(as: .error)]

let package = Package(
    name: "NgtermPlatform",
    platforms: [.iOS(.v26)],
    products: [.library(name: "NgtermPlatform", targets: ["NgtermPlatform"])],
    dependencies: [.package(path: "../NgtermCore")],
    targets: [
        .target(
            name: "NgtermPlatform",
            dependencies: [.product(name: "NgtermCore", package: "NgtermCore")],
            swiftSettings: strict
        ),
        .testTarget(
            name: "NgtermPlatformTests",
            dependencies: [
                "NgtermPlatform",
                .product(name: "NgtermCore", package: "NgtermCore"),
                .product(name: "NgtermCoreFakes", package: "NgtermCore"),
            ],
            swiftSettings: strict
        ),
    ],
    swiftLanguageModes: [.v6]
)
