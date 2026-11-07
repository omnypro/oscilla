import Testing
import CGba

/// The register blocks through the C interface.
struct RegisterTests {
    /// A square wave of 64 samples between 100 and -100.
    static let buffer: [UInt8] = (0..<8192).map { $0 % 64 < 32 ? 100 : UInt8(bitPattern: -100) }
    static let fifoRom = Support.rom(words: MachineTests.fifoProgram) + buffer

    final class Answers {
        var machine: OpaquePointer?
        var values: [Int] = []
        var names: [UnsafePointer<CChar>?] = []
    }

    static func block(_ m: OpaquePointer, _ index: Int32) -> [UInt8] {
        var out = [UInt8](repeating: 0xEE, count: cgba_register_block_size(m, index))
        #expect(cgba_register_block_get(m, index, &out, out.count) == out.count)
        return out
    }

    static func all(_ m: OpaquePointer) -> [[UInt8]] {
        (0..<cgba_register_block_count(m)).map { block(m, $0) }
    }

    @Test func theBlocksHaveNamesSizesAndBases() throws {
        let m = try #require(Support.machine(rom: Self.fifoRom))
        defer { cgba_free(m) }
        let table: [(String, UInt32, Int)] = [
            ("Sound", 0x0400_0060, 0x2A), ("Wave RAM", 0x0400_0090, 32), ("FIFO", 0x0400_00A0, 8),
            ("Timers", 0x0400_0100, 16), ("DMA 1 and 2", 0x0400_00BC, 24),
        ]
        #expect(cgba_register_block_count(m) == 5)
        for (i, (name, base, size)) in table.enumerated() {
            let index = Int32(i)
            #expect(String(cString: cgba_register_block_name(m, index)) == name)
            #expect(cgba_register_block_name(m, index) == cgba_register_block_name(m, index), "the same pointer each call")
            #expect(cgba_register_block_base(m, index) == base)
            #expect(cgba_register_block_size(m, index) == size)
        }
        #expect(cgba_register_block_name(m, 5) == nil && cgba_register_block_name(m, -1) == nil)
        #expect(cgba_register_block_base(m, 5) == 0 && cgba_register_block_base(m, -1) == 0)
        #expect(cgba_register_block_size(m, 5) == 0 && cgba_register_block_size(m, -1) == 0)
    }

    @Test func theSoundBlockHoldsTheValuesTheProgramStored() throws {
        let m = try #require(Support.machine(rom: Support.rom(words: SoundTests.squareProgram)))
        defer { cgba_free(m) }
        var bias = [UInt8](repeating: 0, count: 0x2A)
        bias[0x29] = 0x02
        #expect(Self.block(m, 0) == bias, "the bias that the BIOS leaves, before the program runs")
        var out = [Int16](repeating: 0, count: 2 * 2000)
        cgba_render_s16(m, &out, 2000)
        var sound = bias
        for (offset, value) in [(0x62, 0xF080), (0x64, 0x8400), (0x80, 0x1177), (0x82, 0x0002), (0x84, 0x0080)] {
            sound[offset - 0x60] = UInt8(value & 0xFF)
            sound[offset - 0x5F] = UInt8(value >> 8)
        }
        #expect(Self.block(m, 0) == sound)
        #expect(cgba_peek(m, 0x0400_0064, 2) == 0, "the register reads back with only its length flag")
    }

    @Test func theFifoTimerAndDmaBlocksHoldTheStoredValues() throws {
        let m = try #require(Support.machine(rom: Self.fifoRom))
        defer { cgba_free(m) }
        var out = [Int16](repeating: 0, count: 2 * 4000)
        cgba_render_s16(m, &out, 4000)
        let fifo = Self.block(m, 2)
        #expect(Array(fifo[0..<4]) == [100, 100, 100, 100] || Array(fifo[0..<4]) == [156, 156, 156, 156])
        #expect(Array(fifo[4..<8]) == [0, 0, 0, 0])
        var timers = [UInt8](repeating: 0, count: 16)
        timers[1] = 0xFE; timers[2] = 0x80
        #expect(Self.block(m, 3) == timers)
        var dma = [UInt8](repeating: 0, count: 24)
        dma.replaceSubrange(0..<4, with: Support.le32(0x0800_0048))
        dma.replaceSubrange(4..<8, with: Support.le32(0x0400_00A0))
        dma.replaceSubrange(8..<12, with: [0x04, 0x00, 0x40, 0xB6])
        #expect(Self.block(m, 4) == dma)
        #expect(Self.block(m, 1) == [UInt8](repeating: 0, count: 32))
    }

    @Test func shortBuffersAndInvalidArgumentsGetNothingOrAPrefix() throws {
        let m = try #require(Support.machine(rom: Self.fifoRom))
        defer { cgba_free(m) }
        var render = [Int16](repeating: 0, count: 2 * 2000)
        cgba_render_s16(m, &render, 2000)
        let all = Self.block(m, 4)
        var short = [UInt8](repeating: 0xEE, count: 5)
        #expect(cgba_register_block_get(m, 4, &short, 5) == 5)
        #expect(short == Array(all[0..<5]))
        var big = [UInt8](repeating: 0xEE, count: 40)
        #expect(cgba_register_block_get(m, 4, &big, 40) == 24)
        #expect(Array(big[0..<24]) == all && big[24...].allSatisfy { $0 == 0xEE })
        var out = [UInt8](repeating: 0xEE, count: 8)
        #expect(cgba_register_block_get(m, 4, &out, 0) == 0)
        #expect(cgba_register_block_get(m, 4, nil, 8) == 0)
        #expect(cgba_register_block_get(m, 5, &out, 8) == 0)
        #expect(cgba_register_block_get(m, -1, &out, 8) == 0)
        #expect(out.allSatisfy { $0 == 0xEE })
        #expect(cgba_register_block_count(nil) == 0)
        #expect(cgba_register_block_name(nil, 0) == nil)
        #expect(cgba_register_block_base(nil, 0) == 0)
        #expect(cgba_register_block_size(nil, 0) == 0)
        #expect(cgba_register_block_get(nil, 0, &out, 8) == 0)
    }

    @Test func nothingHereWorksFromInsideACallback() throws {
        let m = try #require(Support.machine(rom: Self.fifoRom))
        defer { cgba_free(m) }
        let box = Answers()
        box.machine = m
        var observer = cgba_observer(
            context: Unmanaged.passUnretained(box).toOpaque(),
            sound_write: { ctx, _, _, _ in
                let box = Unmanaged<Answers>.fromOpaque(ctx!).takeUnretainedValue()
                var out = [UInt8](repeating: 0xEE, count: 24)
                box.values.append(Int(cgba_register_block_count(box.machine)))
                box.values.append(Int(cgba_register_block_base(box.machine, 0)))
                box.values.append(cgba_register_block_size(box.machine, 0))
                box.values.append(cgba_register_block_get(box.machine, 4, &out, 24))
                box.names.append(cgba_register_block_name(box.machine, 0))
            },
            fifo_sample: nil)
        cgba_set_observer(m, &observer)
        cgba_run_cycles(m, 512 * 100)
        cgba_set_observer(m, nil)
        #expect(!box.values.isEmpty)
        #expect(box.values.allSatisfy { $0 == 0 })
        #expect(box.names.allSatisfy { $0 == nil })
        #expect(cgba_register_block_count(m) == 5, "outside the callback the calls answer again")
    }

    @Test func readingTheBlocksDoesNotChangeTheSound() throws {
        let a = try #require(Support.machine(rom: Self.fifoRom, rate: 48000))
        let b = try #require(Support.machine(rom: Self.fifoRom, rate: 48000))
        defer { cgba_free(a); cgba_free(b) }
        var plain: [Int16] = [], read: [Int16] = []
        var chunk = [Int16](repeating: 0, count: 2 * 500)
        for _ in 0..<20 {
            cgba_render_s16(a, &chunk, 500)
            plain += chunk
            cgba_render_s16(b, &chunk, 500)
            read += chunk
            for i in 0..<cgba_register_block_count(b) {
                _ = Self.block(b, i)
                _ = cgba_register_block_name(b, i)
                _ = cgba_register_block_base(b, i)
            }
        }
        #expect(plain == read)
    }

    @Test func aRestoreBringsEveryBlockBack() throws {
        let m = try #require(Support.machine(rom: Self.fifoRom, rate: 48000))
        defer { cgba_free(m) }
        let start = try #require(cgba_snapshot_take(m))
        defer { cgba_snapshot_free(start) }
        let before = Self.all(m)
        var out = [Int16](repeating: 0, count: 2 * 4000)
        cgba_render_s16(m, &out, 4000)
        let after = Self.all(m)
        #expect(after != before)
        #expect(cgba_snapshot_restore(m, start) == 0)
        #expect(Self.all(m) == before)
        cgba_render_s16(m, &out, 4000)
        #expect(Self.all(m) == after)
        cgba_reset(m)
        #expect(Self.all(m) == before)
    }
}
