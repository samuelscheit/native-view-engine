// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "NveSwiftUI",
    platforms: [
        .iOS(.v16),
        .macOS(.v13),
        .tvOS(.v16),
        .watchOS(.v9),
        .visionOS(.v1),
    ],
    products: [
        .library(
            name: "NveSwiftUI",
            targets: ["NveSwiftUI"]
        ),
    ],
    dependencies: [
        // Static host runtime + the generated C ABI (`nve codegen abi`), shared by every Apple host.
        .package(path: "../../packages/core/host-swift"),
    ],
    targets: [
        .target(
            name: "NveSwiftUI",
            dependencies: [
                .product(name: "CNveAbi", package: "host-swift"),
                .product(name: "NveHost", package: "host-swift"),
                .product(name: "NveHostAbi", package: "host-swift"),
            ],
            path: "Sources/NveSwiftUI"
        ),
        .testTarget(
            name: "NveSwiftUITests",
            dependencies: [
                "NveSwiftUI",
                .product(name: "CNveAbi", package: "host-swift"),
                .product(name: "NveHost", package: "host-swift"),
                .product(name: "NveHostAbi", package: "host-swift"),
            ],
            path: "Tests/NveSwiftUITests"
        ),
    ]
)
