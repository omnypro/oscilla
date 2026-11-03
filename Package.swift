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
        // The NES core, in the same Rust library.
        .library(name: "CNes", type: .static, targets: ["CNes"]),
        .executable(name: "oscilla-gsf", targets: ["OscillaGSF"]),
        .executable(name: "oscilla-gbs", targets: ["OscillaGBS"]),
        .executable(name: "oscilla-nsf", targets: ["OscillaNSF"]),
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
        .target(
            name: "CNes",
            dependencies: ["OscillaRust"],
            path: "swift/CNes"
        ),
        // The Rust library as a prebuilt XCFramework (scripts/build-xcframework.sh).
        .binaryTarget(
            name: "OscillaRust",
            url: "https://github.com/omnypro/oscilla/releases/download/v0.3.5/OscillaRust-0.3.5.xcframework.zip",
            checksum: "2fc837b4348a308404e8dc49821cb0d890f31b1ab0a07bcb146d546be05b0e52"
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
        // Plays an NSF file to a WAV file and writes the stream of sound
        // register stores.
        .executableTarget(
            name: "OscillaNSF",
            dependencies: ["CNes"],
            path: "cores/nes/tools/oscilla-nsf"
        ),
        .testTarget(
            name: "CGbTests",
            dependencies: ["CGb"],
            path: "cores/gb/tests"
        ),
        .testTarget(
            name: "CNesTests",
            dependencies: ["CNes"],
            path: "cores/nes/tests"
        ),
        .testTarget(
            name: "CGbaTests",
            dependencies: ["CGba"],
            path: "cores/gba/tests"
        ),
    ],
    cLanguageStandard: .c99
)
