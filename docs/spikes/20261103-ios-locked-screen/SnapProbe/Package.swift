// swift-tools-version:6.2
import PackageDescription

let package = Package(
    name: "SnapProbe",
    platforms: [.iOS(.v26)],
    products: [.library(name: "SnapProbe", targets: ["SnapProbe"])],
    dependencies: [
        .package(url: "https://github.com/pointfreeco/swift-snapshot-testing", exact: "1.19.7"),
    ],
    targets: [
        .target(name: "SnapProbe"),
        .testTarget(
            name: "SnapProbeTests",
            dependencies: ["SnapProbe", .product(name: "SnapshotTesting", package: "swift-snapshot-testing")]
        ),
    ]
)
