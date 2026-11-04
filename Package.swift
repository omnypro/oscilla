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
        // The SNES core, in the same Rust library.
        .library(name: "CSnes", type: .static, targets: ["CSnes"]),
        .executable(name: "oscilla-gsf", targets: ["OscillaGSF"]),
        .executable(name: "oscilla-gbs", targets: ["OscillaGBS"]),
        .executable(name: "oscilla-nsf", targets: ["OscillaNSF"]),
        .executable(name: "oscilla-spc", targets: ["OscillaSPC"]),
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
        .target(
            name: "CSnes",
            dependencies: ["OscillaRust"],
            path: "swift/CSnes"
        ),
        // The Rust library as a prebuilt XCFramework (scripts/build-xcframework.sh).
        .binaryTarget(
            name: "OscillaRust",
            url: "https://github.com/omnypro/oscilla/releases/download/v0.4.0/OscillaRust-0.4.0.xcframework.zip",
            checksum: "9687e8cb5fb28506e5f0152d6bba54108cbf9e29989bc65ce113c85e88f90412"
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
        // Plays an SPC file to a WAV file and writes the stream of DSP
        // register stores.
        .executableTarget(
            name: "OscillaSPC",
            dependencies: ["CSnes"],
            path: "cores/snes/tools/oscilla-spc"
        ),
        .testTarget(
            name: "CSnesTests",
            dependencies: ["CSnes"],
            path: "cores/snes/tests"
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
