// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Oscilla",
    platforms: [.macOS("15.0"), .iOS("18.0")],
    products: [
        // The emulator core, static so an app links it into its own binary.
        .library(name: "CGba", type: .static, targets: ["CGba"]),
        // The Game Boy core, in the same Rust library.
        .library(name: "CGb", type: .static, targets: ["CGb"]),
        .executable(name: "oscilla-gsf", targets: ["OscillaGSF"]),
        .executable(name: "oscilla-gbs", targets: ["OscillaGBS"]),
    ],
    targets: [
        // The C header of the Rust library, and the module that Swift imports.
        .target(
            name: "CGba",
            dependencies: ["OscillaRust"],
            path: "swift/CGba"
        ),
        .target(
            name: "CGb",
            dependencies: ["OscillaRust"],
            path: "swift/CGb"
        ),
        // The Rust library as a prebuilt XCFramework (scripts/build-xcframework.sh).
        .binaryTarget(
            name: "OscillaRust",
            url: "https://github.com/omnypro/oscilla/releases/download/v0.2.0/OscillaRust-0.2.0.xcframework.zip",
            checksum: "ec2d16d6c55b7b26848e836d1dd10da4b550408e28906534dc5978957ed928c2"
        ),
        // The trace driver: renders a GSF and writes the files that the
        // reference runner writes, so the two can be compared.
        .executableTarget(
            name: "OscillaGSF",
            dependencies: ["CGba"],
            path: "cores/gba/tools/oscilla-gsf"
        ),
        // Plays a GBS file to a WAV file and writes the stream of sound
        // register stores.
        .executableTarget(
            name: "OscillaGBS",
            dependencies: ["CGb"],
            path: "cores/gb/tools/oscilla-gbs"
        ),
        .testTarget(
            name: "CGbTests",
            dependencies: ["CGb"],
            path: "cores/gb/tests"
        ),
        .testTarget(
            name: "CGbaTests",
            dependencies: ["CGba"],
            path: "cores/gba/tests"
        ),
    ],
    cLanguageStandard: .c99
)
