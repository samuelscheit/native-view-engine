// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "NveSwiftUI",
    platforms: [
        .iOS(.v16),
        .macOS(.v13),
    ],
    products: [
        .library(
            name: "NveSwiftUI",
            targets: ["NveSwiftUI"]
        ),
    ],
    targets: [
        .target(
            name: "CNveAbi",
            path: "Sources/CNveAbi",
            publicHeadersPath: "include"
        ),
        .target(
            name: "NveSwiftUI",
            dependencies: ["CNveAbi"],
            path: "Sources/NveSwiftUI"
        ),
        .testTarget(
            name: "NveSwiftUITests",
            dependencies: ["NveSwiftUI", "CNveAbi"],
            path: "Tests/NveSwiftUITests"
        ),
    ]
)
