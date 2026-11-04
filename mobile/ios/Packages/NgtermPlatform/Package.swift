// swift-tools-version: 6.2

// The iOS app's platform services: the Keychain key vault and host keys, the JSON stores, the network monitor,
// notifications, the lifecycle and background refresh.

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
        )
    ],
    swiftLanguageModes: [.v6]
)
