import Testing
import CGba

/// Small cartridges assembled from ARM words. tools/ has no source for them;
/// each program is listed beside its test. They check the paths that a sound
/// driver depends on: the interrupt trampoline with VBlankIntrWait, and a
/// timer that feeds a FIFO through sound DMA.
struct MachineTests {
    /// Installs an interrupt handler, enables the VBlank interrupt and calls
    /// VBlankIntrWait in a loop. The loop counts returns at 0x03000000. The handler
    /// acknowledges IF, records the flag for the BIOS at 0x03007FF8 and counts
    /// interrupts at 0x03000004.
    static let vblankProgram: [UInt32] = [
        0xE3A0_0301, 0xE28F_1038, 0xE3A0_2403, 0xE282_2C7F, 0xE582_10FC, 0xE3A0_1008,
        0xE1C0_10B4, 0xE280_3C02, 0xE3A0_1001, 0xE1C3_10B0, 0xE1C3_10B8, 0xE3A0_4000,
        0xEF05_0000, 0xE284_4001, 0xE3A0_5403, 0xE585_4000, 0xEAFF_FFFA, 0xE280_1C02,
        0xE1D1_20B2, 0xE1C1_20B2, 0xE3A0_3403, 0xE593_C004, 0xE28C_C001, 0xE583_C004,
        0xE283_3C7F, 0xE1D3_CFB8, 0xE18C_C002, 0xE1C3_CFB8, 0xE12F_FF1E,
    ]

    @Test func vblankIntrWaitReturnsOncePerFrame() throws {
        let m = try #require(Support.machine(rom: Support.rom(words: Self.vblankProgram)))
        defer { cgba_free(m) }
        cgba_run_cycles(m, 10 * 280_896)
        #expect(cgba_peek(m, 0x0300_0004, 4) == 10, "one VBlank interrupt per frame")
        #expect(cgba_peek(m, 0x0300_0000, 4) == 10, "VBlankIntrWait returns after each one")
        #expect(cgba_peek(m, 0x0300_7FF8, 2) & 1 == 0, "the wait clears the flag it waited for")
    }

    /// Turns sound on, points DMA1 at a buffer that follows the program
    /// (sound FIFO timing, repeat, 32-bit), and runs timer 0 with a period
    /// of 512 cycles.
    static let fifoProgram: [UInt32] = [
        0xE3A0_0301, 0xE3A0_1080, 0xE1C0_18B4, 0xE59F_1024, 0xE1C0_18B2, 0xE28F_102C,
        0xE580_10BC, 0xE59F_1018, 0xE580_10C0, 0xE59F_1014, 0xE580_10C4, 0xE59F_1010,
        0xE580_1100, 0xEAFF_FFFE, 0x0000_0B04, 0x0400_00A0, 0xB640_0004, 0x0080_FE00,
    ]

    final class Samples { var a: [Int8] = []; var writes = 0 }

    @Test func soundDmaFeedsTheFifoInOrder() throws {
        let buffer: [UInt8] = (0..<256).map { UInt8(truncatingIfNeeded: $0 * 7 + 3) }
        let m = try #require(Support.machine(rom: Support.rom(words: Self.fifoProgram) + buffer))
        defer { cgba_free(m) }
        let box = Samples()
        let context = Unmanaged.passUnretained(box).toOpaque()
        var observer = cgba_observer(
            context: context,
            sound_write: { ctx, _, _, _ in
                Unmanaged<Samples>.fromOpaque(ctx!).takeUnretainedValue().writes += 1
            },
            fifo_sample: { ctx, channel, sample in
                if channel == 0 { Unmanaged<Samples>.fromOpaque(ctx!).takeUnretainedValue().a.append(sample) }
            })
        cgba_set_observer(m, &observer)
        cgba_run_cycles(m, 512 * 300)
        cgba_set_observer(m, nil)
        let expected = buffer.prefix(256).map { Int8(bitPattern: $0) }
        try #require(box.a.count >= 256)
        #expect(Array(box.a.prefix(256)) == expected)
        #expect(box.writes > 64, "each DMA word to the FIFO is reported as a store")
    }

    @Test func renderingIsDeterministicAndResetRepeatsIt() throws {
        let buffer: [UInt8] = (0..<4096).map { UInt8(truncatingIfNeeded: ($0 * $0) >> 3) }
        let rom = Support.rom(words: Self.fifoProgram) + buffer
        let a = try #require(Support.machine(rom: rom, rate: 48000))
        let b = try #require(Support.machine(rom: rom, rate: 48000))
        defer { cgba_free(a); cgba_free(b) }
        var first = [Int16](repeating: 0, count: 2 * 9600)
        var second = first, third = first
        cgba_render_s16(a, &first, 9600)
        cgba_render_s16(b, &second, 9600)
        cgba_reset(a)
        cgba_render_s16(a, &third, 9600)
        #expect(first == second)
        #expect(first == third)
        #expect(first.contains { $0 != first[0] }, "the FIFO output is audible")
    }

    @Test func floatAndIntegerRendersAgree() throws {
        let buffer: [UInt8] = (0..<1024).map { UInt8(truncatingIfNeeded: $0 * 13) }
        let rom = Support.rom(words: Self.fifoProgram) + buffer
        let a = try #require(Support.machine(rom: rom, rate: 44100))
        let b = try #require(Support.machine(rom: rom, rate: 44100))
        defer { cgba_free(a); cgba_free(b) }
        var ints = [Int16](repeating: 0, count: 2 * 4410)
        var floats = [Float](repeating: 0, count: 2 * 4410)
        cgba_render_s16(a, &ints, 4410)
        cgba_render_f32(b, &floats, 4410)
        // The integer render clamps at full scale and the float one does
        // not, so samples at the rails are left out.
        var compared = 0
        for i in 0..<ints.count where abs(Int(ints[i])) < 32000 {
            #expect(abs(Float(ints[i]) / 32768 - floats[i]) < 1.0 / 16384)
            compared += 1
        }
        #expect(compared > ints.count / 2)
    }

    @Test func vblanksComeOncePerFrame() throws {
        let m = try #require(Support.machine(rom: Support.rom(words: [0xEAFF_FFFE])))
        defer { cgba_free(m) }
        let first = cgba_next_vblank(m)
        #expect(first == 41004, "the BIOS hands over 884 cycles into line 126")
        cgba_run_cycles(m, first + 10)
        #expect(cgba_peek(m, 0x0400_0006, 2) == 160)
        #expect(cgba_next_vblank(m) == first + 280_896)
    }

    @Test func outputRateIsChecked() throws {
        let rom = Support.rom(words: [0xEAFF_FFFE])
        #expect(Support.machine(rom: rom, rate: 4000) == nil)
        #expect(Support.machine(rom: rom, rate: 400_000) == nil)
        let m = try #require(Support.machine(rom: rom, rate: 8000))
        cgba_free(m)
    }

    @Test func unknownBiosCallsAreReportedOnce() throws {
        // swi 0x2B (not a function), twice, then park
        let m = try #require(Support.machine(rom: Support.rom(words: [0xEF2B_0000, 0xEF2B_0000, 0xEAFF_FFFE])))
        defer { cgba_free(m) }
        cgba_run_cycles(m, 1000)
        let text = String(cString: cgba_warnings(m))
        #expect(text.contains("0x2B"))
        #expect(text.components(separatedBy: "\n").filter { !$0.isEmpty }.count == 1)
        #expect(cgba_next_pc(m) == 0x0800_0008)
    }
}
