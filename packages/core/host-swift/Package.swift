// swift-tools-version:6.0
import PackageDescription

// Zero-dependency NVE host runtime for Swift: XXH64 / Key64, NveBinValue codec, table & scene readers.
// Swift 6 language mode: strict concurrency checking is on for every target.
let package = Package(
    name: "NveHost",
    platforms: [.macOS(.v13), .iOS(.v16), .tvOS(.v16), .watchOS(.v9), .visionOS(.v1)],
    products: [
        .library(name: "NveHost", targets: ["NveHost"]),
        // The generated C ABI (`nve codegen abi` -> host-cxx), imported in place by its module map.
        .library(name: "CNveAbi", targets: ["CNveAbi"]),
    ],
    targets: [
        .target(name: "NveHost"),
        .target(name: "CNveAbi"),
        // Test-only C mirror of the generated ABI structs (proves C-import protocol conformance).
        .target(name: "CNveHostTestABI", path: "Tests/CNveHostTestABI"),
        .testTarget(name: "NveHostTests", dependencies: ["NveHost", "CNveAbi", "CNveHostTestABI"]),
    ],
    swiftLanguageModes: [.v6]
)
