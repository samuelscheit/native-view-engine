// swift-tools-version:6.0
import PackageDescription

// Zero-dependency NVE host runtime for Swift: XXH64 / Key64, NveBinValue codec, table & scene readers,
// the hybrid FFI runtime, and (`NveHostAbi`) the C `NveValue` graphs hybrid calls borrow.
// Swift 6 language mode: strict concurrency checking is on for every target.
let package = Package(
    name: "NveHost",
    platforms: [.macOS(.v13), .iOS(.v16), .tvOS(.v16), .watchOS(.v9), .visionOS(.v1)],
    products: [
        .library(name: "NveHost", targets: ["NveHost"]),
        // The generated C ABI (`nve codegen abi` -> host-cxx), imported in place by its module map.
        .library(name: "CNveAbi", targets: ["CNveAbi"]),
        // `NveHost` parts typed by the generated C ABI; `NveHost` itself stays pure Swift.
        .library(name: "NveHostAbi", targets: ["NveHostAbi"]),
    ],
    targets: [
        .target(name: "NveHost"),
        .target(name: "CNveAbi"),
        .target(name: "NveHostAbi", dependencies: ["NveHost", "CNveAbi"]),
        // Test-only C mirror of the generated ABI structs (proves C-import protocol conformance).
        .target(name: "CNveHostTestABI", path: "Tests/CNveHostTestABI"),
        .testTarget(name: "NveHostTests", dependencies: ["NveHost", "NveHostAbi", "CNveAbi", "CNveHostTestABI"]),
    ],
    swiftLanguageModes: [.v6]
)
