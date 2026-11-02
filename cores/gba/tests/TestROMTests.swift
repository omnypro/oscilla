import Foundation
import Testing
import CGba

/// Public GBA test cartridges, which are not part of this repository. Point
/// GBA_TESTS_DIR at a folder that holds arm/arm.gba, thumb/thumb.gba,
/// memory/memory.gba and bios/bios.gba to run them; without it these tests
/// are skipped. Each cartridge leaves 0 in its result register when every
/// test passes, or the number of the first failure.
struct TestROMTests {
    static let directory: URL? = ProcessInfo.processInfo.environment["GBA_TESTS_DIR"]
        .map { URL(fileURLWithPath: $0) }

    @Test(.enabled(if: directory != nil, "set GBA_TESTS_DIR to a folder of test cartridges"),
          arguments: [("arm/arm.gba", Int32(12)), ("thumb/thumb.gba", 7),
                      ("memory/memory.gba", 12), ("bios/bios.gba", 12)])
    func cartridgePasses(_ path: String, _ register: Int32) throws {
        let url = try #require(Self.directory).appendingPathComponent(path)
        let result = try Support.runTestROM(at: url, resultRegister: register)
        #expect(result == 0, "\(path): first failing test \(result.map(String.init) ?? "none, it never finished")")
    }
}
