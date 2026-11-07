import Foundation
import Testing
import CSnes

/// The register blocks, the echo scope, the position in a sample, the decoded
/// samples and the timers.
@Suite("Inspection")
struct InspectionTests {
    /// One BRR block from a header and sixteen nibbles.
    static func brr(shift: UInt8, filter: UInt8, flags: UInt8, _ nibbles: [UInt8]) -> [UInt8] {
        var bytes = [shift << 4 | filter << 2 | flags]
        for pair in stride(from: 0, to: 16, by: 2) { bytes.append(nibbles[pair] << 4 | nibbles[pair + 1]) }
        return bytes
    }

    static func put(_ bytes: [UInt8], at address: Int) -> [(Int, UInt8)] {
        bytes.enumerated().map { (address + $0.offset, $0.element) }
    }

    static let ramp: [UInt8] = [1, 7, 8, 15, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
    static let ones = [UInt8](repeating: 1, count: 16)

    /// Voice 0 loops over the second block of a two-block sample, with the echo
    /// returning the voice.
    static func echoing() -> [UInt8] {
        let ram = put([0x00, 0x20, 0x09, 0x20], at: 0x1000)
            + put(brr(shift: 12, filter: 0, flags: 0, [UInt8](repeating: 7, count: 16)), at: 0x2000)
            + put(brr(shift: 11, filter: 2, flags: 3, [UInt8](repeating: 7, count: 16)), at: 0x2009)
        let dsp: [(Int, UInt8)] = [
            (0x00, 0x20), (0x01, 0x20), (0x02, 0x00), (0x03, 0x10), (0x04, 0x00), (0x05, 0x00),
            (0x07, 0x40), (0x0C, 0x7F), (0x1C, 0x7F), (0x5D, 0x10), (0x4C, 0x01),
            (0x4D, 0x01), (0x6D, 0x30), (0x7D, 0x01), (0x6C, 0x00), (0x0F, 0x7F),
            (0x2C, 0x40), (0x3C, 0x20),
        ]
        // Timer 2 on with the divisor 1, then a spin.
        return Support.spc(program: [0x8F, 0x01, 0xFC, 0x8F, 0x04, 0xF1, 0x2F, 0xFE], dsp: dsp, ram: ram)
    }

    /// Four blocks with the four filters, ending with a loop back to the second.
    static func filters() -> [UInt8] {
        let ram = put([0x00, 0x20, 0x09, 0x20, 0x00, 0x80, 0x00, 0x80], at: 0x1000)
            + put(brr(shift: 4, filter: 0, flags: 0, ramp), at: 0x2000)
            + put(brr(shift: 2, filter: 1, flags: 0, ones), at: 0x2009)
            + put(brr(shift: 3, filter: 2, flags: 0, [3, 5, 9, 12, 0, 7, 8, 1, 2, 14, 15, 4, 6, 10, 11, 13]), at: 0x2012)
            + put(brr(shift: 3, filter: 3, flags: 3, [7, 7, 8, 8, 6, 5, 9, 10, 0, 1, 2, 3, 12, 13, 14, 15]), at: 0x201B)
        return Support.spc(program: [0x2F, 0xFE], dsp: [(0x5D, 0x10)], ram: ram)
    }

    @Test func theStructsHaveTheDocumentedLayout() {
        #expect(MemoryLayout<csnes_brr>.size == 24)
        #expect(MemoryLayout<csnes_brr>.offset(of: \.directory_entry_address) == 0)
        #expect(MemoryLayout<csnes_brr>.offset(of: \.block_address) == 2)
        #expect(MemoryLayout<csnes_brr>.offset(of: \.loop_address) == 4)
        #expect(MemoryLayout<csnes_brr>.offset(of: \.sample_in_block) == 6)
        #expect(MemoryLayout<csnes_brr>.offset(of: \.srcn) == 7)
        #expect(MemoryLayout<csnes_brr>.offset(of: \.loops_done_saturating) == 8)
        #expect(MemoryLayout<csnes_brr>.offset(of: \.flags) == 9)
        #expect(MemoryLayout<csnes_brr>.offset(of: \.filter) == 10)
        #expect(MemoryLayout<csnes_brr>.offset(of: \.range) == 11)
        #expect(MemoryLayout<csnes_brr>.offset(of: \.loop_length_samples) == 12)
        #expect(MemoryLayout<csnes_brr>.offset(of: \.total_samples) == 16)
        #expect(MemoryLayout<csnes_sample_info>.size == 16)
        #expect(MemoryLayout<csnes_sample_info>.offset(of: \.total_samples) == 4)
        #expect(MemoryLayout<csnes_sample_info>.offset(of: \.loop_start_sample) == 8)
        #expect(MemoryLayout<csnes_sample_info>.offset(of: \.has_loop) == 12)
        #expect(MemoryLayout<csnes_timer>.size == 8)
        #expect(MemoryLayout<csnes_timer>.offset(of: \.ticks_until_output) == 4)
    }

    @Test func theRegisterBlocksNameBaseAndSizeTheChipsRegisters() {
        let m = Support.machine(InspectionTests.echoing())
        defer { csnes_free(m) }
        _ = Support.render(m, frames: 700)
        #expect(csnes_register_block_count(m) == 2)
        #expect(String(cString: csnes_register_block_name(m, 0)) == "S-DSP")
        #expect(String(cString: csnes_register_block_name(m, 1)) == "SPC700 I/O")
        #expect(csnes_register_block_name(m, 0) == csnes_register_block_name(m, 0))
        #expect(csnes_register_block_base(m, 0) == 0x00 && csnes_register_block_size(m, 0) == 128)
        #expect(csnes_register_block_base(m, 1) == 0xF0 && csnes_register_block_size(m, 1) == 16)

        var dsp = [UInt8](repeating: 0, count: 128)
        #expect(csnes_register_block_get(m, 0, &dsp, 128) == 128)
        for r in 0..<128 { #expect(UInt32(dsp[r]) == csnes_dsp_register(m, UInt32(r))) }
        #expect(dsp[0x6D] == 0x30)
        #expect(dsp[0x08] == 0x40)
        #expect(dsp[0x09] != 0)

        var io = [UInt8](repeating: 0, count: 16)
        #expect(csnes_register_block_get(m, 1, &io, 16) == 16)
        #expect(io[0] == 0 && io[1] == 0)
        #expect(io[0xC] == 1)
        #expect(io[0xF] != 0)
        // A read clears nothing.
        var again = [UInt8](repeating: 0, count: 16)
        #expect(csnes_register_block_get(m, 1, &again, 16) == 16)
        #expect(io == again)

        var short = [UInt8](repeating: 0xAA, count: 8)
        #expect(csnes_register_block_get(m, 1, &short, 4) == 4)
        #expect(Array(short[0..<4]) == Array(io[0..<4]) && short[4] == 0xAA)
        var long = [UInt8](repeating: 0xAA, count: 300)
        #expect(csnes_register_block_get(m, 0, &long, 300) == 128)
        #expect(long[128] == 0xAA)
    }

    @Test func aBadRegisterBlockCallReturnsNothing() {
        let m = Support.machine(InspectionTests.echoing())
        defer { csnes_free(m) }
        var out = [UInt8](repeating: 0x55, count: 16)
        #expect(csnes_register_block_get(m, 2, &out, 16) == 0)
        #expect(csnes_register_block_get(m, -1, &out, 16) == 0)
        #expect(csnes_register_block_get(m, 0, nil, 16) == 0)
        #expect(csnes_register_block_get(m, 0, &out, 0) == 0)
        #expect(out.allSatisfy { $0 == 0x55 })
        #expect(csnes_register_block_name(m, 2) == nil && csnes_register_block_name(m, -1) == nil)
        #expect(csnes_register_block_base(m, 2) == 0 && csnes_register_block_size(m, 2) == 0)
        #expect(csnes_register_block_count(nil) == 0)
        #expect(csnes_register_block_name(nil, 0) == nil)
        #expect(csnes_register_block_base(nil, 0) == 0 && csnes_register_block_size(nil, 0) == 0)
        #expect(csnes_register_block_get(nil, 0, &out, 16) == 0)
    }

    @Test func theEchoScopeShowsWhatTheEchoAddsAndRefusesBadCalls() {
        let m = Support.machine(InspectionTests.echoing())
        defer { csnes_free(m) }
        var scope = [Float](repeating: 1, count: 300)
        #expect(csnes_echo_scope_get(m, 0, &scope, 300) == 300)
        #expect(scope.allSatisfy { $0 == 0 })
        _ = Support.render(m, frames: 2500)
        var left = [Float](repeating: 0, count: 800)
        var right = [Float](repeating: 0, count: 800)
        #expect(csnes_echo_scope_get(m, 0, &left, 800) == 800)
        #expect(csnes_echo_scope_get(m, 1, &right, 800) == 800)
        #expect(left.contains { $0 != 0 } && right.contains { $0 != 0 })
        #expect((left + right).allSatisfy { $0 >= -1 && $0 <= 1 })
        // The right volume is half the left one.
        let l = left.reduce(0) { $0 + abs($1) }
        let r = right.reduce(0) { $0 + abs($1) }
        #expect(abs(r * 2 - l) < l * 0.02)
        // The mask leaves it alone.
        csnes_set_channel_mask(m, 0xFF)
        var masked = [Float](repeating: 0, count: 1)
        #expect(csnes_echo_scope_get(m, 0, &masked, 1) == 1)
        #expect(csnes_echo_scope_get(m, 2, &scope, 64) == 0)
        #expect(csnes_echo_scope_get(m, -1, &scope, 64) == 0)
        #expect(csnes_echo_scope_get(m, 0, nil, 64) == 0)
        #expect(csnes_echo_scope_get(m, 0, &scope, 0) == 0)
        #expect(csnes_echo_scope_get(m, 0, &scope, 16_385) == 0)
        #expect(csnes_echo_scope_get(nil, 0, &scope, 64) == 0)
    }

    @Test func aVoiceReportsWhereItIsInItsSample() {
        let m = Support.machine(InspectionTests.echoing())
        defer { csnes_free(m) }
        _ = Support.render(m, frames: 2000)
        var p = csnes_brr()
        #expect(csnes_voice_brr_get(m, 0, &p) == 1)
        #expect(p.directory_entry_address == 0x1000)
        #expect(p.loop_address == 0x2009)
        #expect(p.block_address == 0x2009)
        #expect(p.srcn == 0)
        #expect(p.flags == 3 && p.filter == 2 && p.range == 11)
        #expect(p.sample_in_block <= 15)
        #expect(p.loops_done_saturating > 30)
        #expect(p.total_samples == 32 && p.loop_length_samples == 16)
        #expect(p.reserved.0 == 0 && p.reserved.3 == 0)
        // The sample position moves with the clock.
        var q = csnes_brr()
        _ = Support.render(m, frames: 5)
        #expect(csnes_voice_brr_get(m, 0, &q) == 1)
        #expect(q.sample_in_block != p.sample_in_block || q.block_address != p.block_address)
        // A voice that was never keyed on has no block.
        var idle = csnes_brr()
        #expect(csnes_voice_brr_get(m, 4, &idle) == 1)
        #expect(idle.block_address == 0 && idle.loops_done_saturating == 0 && idle.flags == 0)
        #expect(idle.total_samples == 32)
        #expect(csnes_voice_brr_get(m, 8, &p) == 0)
        #expect(csnes_voice_brr_get(m, -1, &p) == 0)
        #expect(csnes_voice_brr_get(m, 0, nil) == 0)
        #expect(csnes_voice_brr_get(nil, 0, &p) == 0)
    }

    @Test func aSampleDecodesWithEachFilterAsTheDocumentationComputesIt() {
        let m = Support.machine(InspectionTests.filters())
        defer { csnes_free(m) }
        // Worked by hand from the BRR formulas of fullsnes: block 1 (range 4,
        // filter 0) gives 8 * nibble, block 2 (range 2, filter 1) starts at -36,
        // block 3 (range 3, filter 2) at 14 and block 4 (range 3, filter 3) at -111.
        let expected: [Int16] = [
            8, 56, -64, -8, 0, 8, 16, 24, 32, 40, 48, 56, -64, -56, -48, -40,
            -36, -32, -28, -25, -22, -19, -16, -13, -11, -9, -7, -5, -3, -1, 1, 2,
            14, 44, 41, 20, -1, 7, -19, -40, -52, -71, -92, -94, -70, -70, -89, -117,
            -111, -77, -81, -116, -120, -102, -115, -149, -175, -190, -192, -179, -182, -195, -212, -227,
        ]
        var out = [Int16](repeating: 0, count: 100)
        var info = csnes_sample_info()
        #expect(csnes_sample_get(m, 0, &out, 100, &info) == 64)
        #expect(Array(out[0..<64]) == expected)
        #expect(out[64] == 0)
        #expect(info.start_address == 0x2000 && info.loop_address == 0x2009)
        #expect(info.total_samples == 64 && info.loop_start_sample == 16)
        #expect(info.has_loop == 1 && info.truncated == 0)
        #expect(info.reserved.0 == 0 && info.reserved.1 == 0)

        // A short buffer takes the first samples, and the info shows the whole.
        var few = [Int16](repeating: 0x7777, count: 8)
        #expect(csnes_sample_get(m, 0, &few, 3, &info) == 3)
        #expect(Array(few[0..<4]) == [8, 56, -64, 0x7777])
        #expect(info.total_samples == 64)
        // The info may be NULL.
        #expect(csnes_sample_get(m, 0, &out, 100, nil) == 64)
        #expect(csnes_sample_get(m, 0, &out, 20_000, nil) == 64)

        // Entry 1 points at zeros, so the scan runs the 4,096 blocks and gives up.
        var zeros = [Int16](repeating: 0, count: 70_000)
        #expect(csnes_sample_get(m, 1, &zeros, 70_000, &info) == 65_536)
        #expect(info.truncated == 1 && info.has_loop == 0 && info.total_samples == 65_536)
        var p = csnes_brr()
        #expect(csnes_voice_brr_get(m, 0, &p) == 1)
        #expect(p.total_samples == 64 && p.loop_length_samples == 48)
    }

    @Test func aBadSampleCallWritesNothing() {
        let m = Support.machine(InspectionTests.filters())
        defer { csnes_free(m) }
        var out = [Int16](repeating: 0x55, count: 32)
        var info = csnes_sample_info()
        info.total_samples = 9
        #expect(csnes_sample_get(m, 256, &out, 32, &info) == 0)
        #expect(csnes_sample_get(m, -1, &out, 32, &info) == 0)
        #expect(csnes_sample_get(m, 0, nil, 32, &info) == 0)
        #expect(csnes_sample_get(m, 0, &out, 0, &info) == 0)
        #expect(csnes_sample_get(nil, 0, &out, 32, &info) == 0)
        #expect(out.allSatisfy { $0 == 0x55 } && info.total_samples == 9)
    }

    @Test func theTimersReportTheirStateWithoutClearingTheOutput() {
        // Timer 0 divides by 3, timer 1 by 256 (a divisor of 0) and timer 2 by 1.
        let program: [UInt8] = [0x8F, 0x03, 0xFA, 0x8F, 0x00, 0xFB, 0x8F, 0x01, 0xFC, 0x8F, 0x05, 0xF1, 0x2F, 0xFE]
        let m = Support.machine(Support.spc(program: program))
        defer { csnes_free(m) }
        csnes_run_cycles(m, 5000)
        var t = [csnes_timer](repeating: csnes_timer(), count: 3)
        #expect(csnes_timers_get(m, &t) == 1)
        #expect(t[0].enabled == 1 && t[0].divisor == 3)
        #expect(t[1].enabled == 0 && t[1].divisor == 0 && t[1].ticks_until_output == 0)
        #expect(t[2].enabled == 1 && t[2].divisor == 1)
        #expect(t[0].ticks_until_output >= 1 && t[0].ticks_until_output <= 3 * 128)
        #expect(t[2].ticks_until_output >= 1 && t[2].ticks_until_output <= 16)
        #expect(t[2].output != 0)
        var again = [csnes_timer](repeating: csnes_timer(), count: 3)
        #expect(csnes_timers_get(m, &again) == 1)
        #expect(again[2].output == t[2].output && again[2].ticks_until_output == t[2].ticks_until_output)
        #expect(t[0].reserved == 0)
        // The output reaches the register block unchanged.
        var io = [UInt8](repeating: 0, count: 16)
        _ = csnes_register_block_get(m, 1, &io, 16)
        #expect(io[0xF] == t[2].output)
        // The output increments once ticks_until_output cycles have passed. The
        // spin loop takes 4 cycles an instruction, so the run ends within 3 cycles
        // after the increment.
        let before = t[2].output
        let start = csnes_cycles(m)
        csnes_run_cycles(m, UInt64(t[2].ticks_until_output))
        #expect(csnes_cycles(m) - start >= UInt64(t[2].ticks_until_output))
        _ = csnes_timers_get(m, &again)
        #expect(again[2].output == (before + 1) & 15)
        #expect(csnes_timers_get(m, nil) == 0)
        #expect(csnes_timers_get(nil, &t) == 0)
    }

    @Test func readingTheStateDoesNotChangeTheSound() {
        func play(reading: Bool) -> [Int16] {
            let m = Support.machine(InspectionTests.echoing())
            defer { csnes_free(m) }
            var all: [Int16] = []
            var scope = [Float](repeating: 0, count: 300)
            var dsp = [UInt8](repeating: 0, count: 128)
            var io = [UInt8](repeating: 0, count: 16)
            var timers = [csnes_timer](repeating: csnes_timer(), count: 3)
            var samples = [Int16](repeating: 0, count: 64)
            for _ in 0..<10 {
                all += Support.render(m, frames: 700)
                if reading {
                    _ = csnes_echo_scope_get(m, 0, &scope, 300)
                    _ = csnes_echo_scope_get(m, 1, &scope, 300)
                    _ = csnes_register_block_get(m, 0, &dsp, 128)
                    _ = csnes_register_block_get(m, 1, &io, 16)
                    _ = csnes_timers_get(m, &timers)
                    for v: Int32 in 0..<8 {
                        var p = csnes_brr()
                        _ = csnes_voice_brr_get(m, v, &p)
                    }
                    for s: Int32 in 0..<8 { _ = csnes_sample_get(m, s, &samples, 64, nil) }
                }
            }
            return all
        }
        let quiet = play(reading: false)
        #expect(quiet == play(reading: true))
        #expect(quiet.contains { $0 != 0 })
    }
}
