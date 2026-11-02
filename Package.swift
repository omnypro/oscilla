// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Oscilla",
    platforms: [.macOS("15.0"), .iOS("18.0")],
    products: [
        // The emulator core, static so an app links it into its own binary.
        .library(name: "CGba", type: .static, targets: ["CGba"]),
        .executable(name: "oscilla-gsf", targets: ["OscillaGSF"]),
    ],
    targets: [
        // The C header of the Rust library, and the module that Swift imports.
        .target(
            name: "CGba",
            dependencies: ["OscillaRust"],
            path: "swift/CGba"
        ),
        // The Rust library as a prebuilt XCFramework (scripts/build-xcframework.sh).
        .binaryTarget(
            name: "OscillaRust",
            path: "build/OscillaRust.xcframework"
        ),
        // The trace driver: renders a GSF and writes the files that the
        // reference runner writes, so the two can be compared.
        .executableTarget(
            name: "OscillaGSF",
            dependencies: ["CGba"],
            path: "cores/gba/tools/oscilla-gsf"
        ),
        .testTarget(
            name: "CGbaTests",
            dependencies: ["CGba"],
            path: "cores/gba/tests"
        ),
    ],
    cLanguageStandard: .c99
)
