// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "PrtrackMenuBar",
    platforms: [.macOS(.v14)],
    targets: [
        .systemLibrary(name: "prtrack_ffiFFI", path: "Sources/prtrack_ffiFFI/include"),
        .target(
            name: "PrtrackFFI",
            dependencies: ["prtrack_ffiFFI"],
            linkerSettings: [
                .unsafeFlags(["-L../../target/release"]),
                .linkedLibrary("prtrack_ffi"),
            ]
        ),
        .executableTarget(name: "PrtrackMenuBar", dependencies: ["PrtrackFFI"]),
    ]
)
