// swift-tools-version:6.0
// The queue-focus engine for Swift. `make mac-core` builds the XCFramework
// and generates Sources/QfCore/QfCore.swift; neither is checked in.
import PackageDescription

let package = Package(
    name: "QfCore",
    platforms: [.macOS(.v14)],
    products: [.library(name: "QfCore", targets: ["QfCore"])],
    targets: [
        .binaryTarget(name: "QfCoreFFI", path: "QfCoreFFI.xcframework"),
        .target(name: "QfCore", dependencies: ["QfCoreFFI"]),
        .testTarget(name: "QfCoreTests", dependencies: ["QfCore"]),
    ]
)
