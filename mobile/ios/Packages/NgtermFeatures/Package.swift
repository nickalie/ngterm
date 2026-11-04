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
        )
    ],
    swiftLanguageModes: [.v6]
)
