import Foundation
import Testing
import CGba

/// Programs and files from outside are untrusted: a guest program must not be
/// able to stall a render, and a GSF must not be able to name files outside
/// its own folder or fan out into a pile of loads.
struct HardeningTests {
    final class Seen { var sentinel = false }

    /// Calls BIOS function `function` with r0 = `source`, r1 = `destination`,
    /// r2 = `count`, r3 = 2, then writes SOUNDCNT_X, so a sound-register
    /// store is the sign that the call returned.
    private static func program(function: UInt32, source: UInt32, destination: UInt32, count: UInt32) -> [UInt32] {
        [
            0xE59F_0034,                 // ldr r0, [pc, #0x34]   source
            0xE59F_1034,                 // ldr r1, [pc, #0x34]   destination
            0xE59F_2034,                 // ldr r2, [pc, #0x34]   count
            0xE3A0_3002,                 // mov r3, #2
            0xEF00_0000 | (function << 16),
            0xE3A0_0301,                 // mov r0, #0x04000000
            0xE3A0_1080,                 // mov r1, #0x80
            0xE1C0_18B4,                 // strh r1, [r0, #0x84]
            0xEAFF_FFFE,                 // b .
            0, 0, 0, 0, 0,
            source, destination, count,
        ]
    }

    private static func returns(from rom: [UInt8], cycles: UInt64 = 1_000_000) throws -> Bool {
        let m = try #require(Support.machine(rom: rom))
        defer { cgba_free(m) }
        let seen = Seen()
        var observer = cgba_observer(
            context: Unmanaged.passUnretained(seen).toOpaque(),
            sound_write: { ctx, address, _, _ in
                if address == 0x0400_0084 { Unmanaged<Seen>.fromOpaque(ctx!).takeUnretainedValue().sentinel = true }
            },
            fifo_sample: nil)
        cgba_set_observer(m, &observer)
        cgba_run_cycles(m, cycles)
        cgba_set_observer(m, nil)
        return seen.sentinel
    }

    @Test func huffmanWithABitWidthThatDoesNotDivideThirtyTwoReturns() throws {
        // Header: 3-bit units, 4 bytes out. Tree: one node with two leaves.
        // The stream is all zero bits, so every unit is the first leaf.
        var rom = Support.rom(words: Self.program(function: 0x13, source: 0x0800_0050, destination: 0x0200_0000, count: 0))
        rom += [UInt8](repeating: 0, count: 0x50 - rom.count)
        rom += [0x23, 0x04, 0x00, 0x00, 0x01, 0xC0, 0x01, 0x02] + [UInt8](repeating: 0, count: 64)
        #expect(try Self.returns(from: rom))
    }

    @Test func huffmanWithATreeThatNeverReachesALeafReturns() throws {
        // The root points back at itself: every bit lands on an inner node.
        var rom = Support.rom(words: Self.program(function: 0x13, source: 0x0800_0050, destination: 0x0200_0000, count: 0))
        rom += [UInt8](repeating: 0, count: 0x50 - rom.count)
        rom += [0x28, 0x40, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00] + [UInt8](repeating: 0, count: 64)
        #expect(try Self.returns(from: rom))
    }

    @Test func affineSetsWithAHugeCountReturn() throws {
        for function: UInt32 in [0x0E, 0x0F] {
            let rom = Support.rom(words: Self.program(function: function, source: 0x0200_0000, destination: 0x0300_0000, count: 0xFFFF_FFFF))
            #expect(try Self.returns(from: rom, cycles: 4_000_000), "function \(function)")
        }
    }

    // MARK: - The loader

    private static func write(_ bytes: [UInt8], _ name: String, in directory: URL) throws -> URL {
        let url = directory.appendingPathComponent(name)
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data(bytes).write(to: url)
        return url
    }

    private static func load(_ url: URL) -> Int32 {
        var image: OpaquePointer?
        var error = [CChar](repeating: 0, count: 512)
        let status = cgba_image_load_gsf(url.path, &image, &error, error.count).rawValue
        if let image { cgba_image_free(image) }
        return Int32(status)
    }

    @Test func aLibraryFanOutIsRefusedInsteadOfLoadedBillionsOfTimes() throws {
        let directory = try Support.scratchDirectory()
        // Eleven files; each names the next one nine times.
        for level in 0...10 {
            let next = "level\(level + 1).gsf"
            let tags = ["_lib=\(next)"] + (2...9).map { "_lib\($0)=\(next)" }
            let tagList = level == 10 ? [] : tags
            _ = try Self.write(Support.gsf(load: 0x0800_0000, data: [0, 0, 0, 0], tags: tagList), "level\(level).gsf", in: directory)
        }
        let status = Self.load(directory.appendingPathComponent("level0.gsf"))
        #expect(status == Int32(CGBA_ERR_LIBRARY.rawValue))
    }

    @Test func aLibraryNameCannotLeaveTheFilesFolder() throws {
        let directory = try Support.scratchDirectory()
        let outside = try Self.write(Support.gsf(load: 0x0800_0000, data: [1, 2, 3, 4]), "outside.gsf", in: directory)
        for name in ["../outside.gsf", outside.path, "sub/../../outside.gsf"] {
            let stub = try Self.write(Support.gsf(load: 0x0800_0000, data: [0, 0, 0, 0], tags: ["_lib=\(name)"]),
                                      "sub/stub-\(UUID().uuidString).gsf", in: directory)
            #expect(Self.load(stub) == Int32(CGBA_ERR_LIBRARY.rawValue), "\(name)")
        }
    }

    @Test func aLibraryInTheSameFolderStillLoads() throws {
        let directory = try Support.scratchDirectory()
        _ = try Self.write(Support.gsf(load: 0x0800_0000, data: [1, 2, 3, 4]), "set.gsflib", in: directory)
        let stub = try Self.write(Support.gsf(load: 0x0800_0004, data: [5, 6, 7, 8], tags: ["_lib=set.gsflib"]), "song.minigsf", in: directory)
        #expect(Self.load(stub) == Int32(CGBA_OK.rawValue))
    }

    @Test func aNilPathReturnsAnError() {
        var image: OpaquePointer?
        var error = [CChar](repeating: 0, count: 64)
        let status = cgba_image_load_gsf(nil, &image, &error, error.count)
        #expect(status != CGBA_OK)
    }
}
