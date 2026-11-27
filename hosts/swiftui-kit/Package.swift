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
        .package(name: "NveHost", path: "../../packages/core/host-swift"),
    ],
    targets: [
        // Generated `nve_abi.generated.h` + `nve_runtime_abi.generated.h` from
        // `packages/core/host-cxx/include/nve/host` (run `cargo run --locked -p nve-cli -- codegen abi`).
        .target(
            name: "CNveAbi",
            path: "Sources/CNveAbi",
            publicHeadersPath: "include"
        ),
        .target(
            name: "NveSwiftUI",
            dependencies: ["CNveAbi", .product(name: "NveHost", package: "NveHost")],
            path: "Sources/NveSwiftUI"
        ),
        .testTarget(
            name: "NveSwiftUITests",
            dependencies: ["NveSwiftUI", "CNveAbi", .product(name: "NveHost", package: "NveHost")],
            path: "Tests/NveSwiftUITests"
        ),
    ]
)