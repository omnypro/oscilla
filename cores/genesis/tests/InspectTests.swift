import Foundation
import Testing
import CGenesis

/// The register blocks and the FM patch view.
@Suite("Inspection")
struct InspectTests {
    /// A file with a YM2413 and no other chip.
    static func opllVgm(stream: [UInt8]) -> [UInt8] {
        var file = Support.vgm(stream: stream, psg: false, fm: false)
        let clock: UInt32 = 3_579_545
        for i in 0..<4 { file[0x10 + i] = UInt8(truncatingIfNeeded: clock >> (8 * UInt32(i))) }
        return file
    }

    static func opll(_ register: UInt8, _ value: UInt8) -> [UInt8] {
        [0x51, register, value]
    }

    static func ops(_ fm: cgenesis_fm) -> [cgenesis_fm_operator] {
        [fm.op.0, fm.op.1, fm.op.2, fm.op.3]
    }

    static func block(_ m: OpaquePointer, _ index: Int32) -> [UInt8] {
        let size = cgenesis_register_block_size(m, index)
        var bytes = [UInt8](repeating: 0xEE, count: size)
        #expect(cgenesis_register_block_get(m, index, &bytes, size) == size)
        return bytes
    }

    @Test func theLayoutsAreFixed() {
        #expect(MemoryLayout<cgenesis_fm_operator>.size == 24)
        #expect(MemoryLayout<cgenesis_fm_operator>.offset(of: \.envelope_level) == 16)
        #expect(MemoryLayout<cgenesis_fm_operator>.offset(of: \.frequency_hz) == 20)
        #expect(MemoryLayout<cgenesis_fm>.size == 104)
        #expect(MemoryLayout<cgenesis_fm>.offset(of: \.op) == 8)
    }

    @Test func thereIsOneBlockForEachChip() {
        let m = Support.machine(Support.vgm(stream: Support.fmTone() + Support.psgTone() + Support.wait(1000)))
        defer { cgenesis_free(m) }
        #expect(cgenesis_register_block_count(m) == 2)
        #expect(String(cString: cgenesis_register_block_name(m, 0)) == "YM2612")
        #expect(String(cString: cgenesis_register_block_name(m, 1)) == "SN76489")
        #expect(cgenesis_register_block_name(m, 0) == cgenesis_register_block_name(m, 0))
        #expect(cgenesis_register_block_base(m, 0) == 0 && cgenesis_register_block_base(m, 1) == 0)
        #expect(cgenesis_register_block_size(m, 0) == 512)
        #expect(cgenesis_register_block_size(m, 1) == 12)

        var both = Support.vgm(stream: Support.wait(10))
        both[0x0F] |= 0x40
        both[0x2F] |= 0x40
        both[0x10 + 3] |= 0x40
        for i in 0..<4 { both[0x10 + i] |= UInt8(truncatingIfNeeded: UInt32(3_579_545) >> (8 * UInt32(i))) }
        let all = Support.machine(both)
        defer { cgenesis_free(all) }
        #expect(cgenesis_register_block_count(all) == 6)
        let names = (0..<6).map { String(cString: cgenesis_register_block_name(all, Int32($0))) }
        #expect(names == ["YM2612", "SN76489", "YM2612 (second)", "SN76489 (second)", "YM2413", "YM2413 (second)"])
        #expect(cgenesis_register_block_size(all, 4) == 64)
    }

    @Test func aBlockOutsideTheChipsReadsAsNothing() {
        let m = Support.machine(Support.vgm(stream: Support.wait(10)))
        defer { cgenesis_free(m) }
        var bytes = [UInt8](repeating: 0xEE, count: 600)
        for block: Int32 in [2, 99, -1] {
            #expect(cgenesis_register_block_name(m, block) == nil)
            #expect(cgenesis_register_block_base(m, block) == 0)
            #expect(cgenesis_register_block_size(m, block) == 0)
            #expect(cgenesis_register_block_get(m, block, &bytes, 600) == 0)
        }
        #expect(cgenesis_register_block_get(m, 0, nil, 600) == 0)
        #expect(cgenesis_register_block_get(m, 0, &bytes, 0) == 0)
        #expect(bytes.allSatisfy { $0 == 0xEE })
        #expect(cgenesis_register_block_count(nil) == 0)
        #expect(cgenesis_register_block_name(nil, 0) == nil)
        #expect(cgenesis_register_block_size(nil, 0) == 0)
        #expect(cgenesis_register_block_get(nil, 0, &bytes, 600) == 0)
        // A short buffer gets the first bytes and nothing past them.
        #expect(cgenesis_register_block_get(m, 1, &bytes, 4) == 4)
        #expect(bytes[4] == 0xEE)
        // A long buffer gets the whole block.
        #expect(cgenesis_register_block_get(m, 1, &bytes, 600) == 12)
        #expect(bytes[12] == 0xEE)
    }

    @Test func theBlocksHoldTheRegistersThatTheFileWrote() {
        let stream = Support.fmTone() + Support.fm(0xB5, 0x67, part: 1) + Support.psgTone()
            + [0x50, 0xE5, 0x4F, 0x5A] + Support.wait(1000)
        let m = Support.machine(Support.vgm(stream: stream))
        defer { cgenesis_free(m) }
        _ = Support.render(m, frames: 100)
        let fm = Self.block(m, 0)
        #expect(fm[0x30] == 0x01 && fm[0x34] == 0x01 && fm[0x40] == 0 && fm[0x44] == 127)
        #expect(fm[0xB0] == 7 && fm[0xB4] == 0xC0 && fm[0xA4] == 0x22 && fm[0xA0] == 0x4B)
        #expect(fm[0x1B5] == 0x67 && fm[0x1B4] == 0xC0)
        #expect(fm[0x28] == 0 && fm[0x2A] == 0x80)
        let psg = Self.block(m, 1)
        #expect(Array(psg[0..<2]) == [0xFE, 0x00])
        #expect(psg[6] == 5 && psg[7] == 0 && psg[8] == 15 && psg[11] == 0x5A)
    }

    @Test func aYm2612PatchHasItsOperatorsInOrder() {
        // Multiples 1, 3, 2, 4 at the register slots are operators 1, 2, 3, 4
        // with multiples 1, 2, 3, 4.
        var stream: [UInt8] = []
        for (offset, multiple, level) in [(0, 1, 0x11), (4, 3, 0x33), (8, 2, 0x22), (12, 4, 0x44)] as [(UInt8, UInt8, UInt8)] {
            stream += Support.fm(0x30 + offset, multiple) + Support.fm(0x40 + offset, level)
            stream += Support.fm(0x50 + offset, 0x1F) + Support.fm(0x60 + offset, 0x80)
            stream += Support.fm(0x70 + offset, 0) + Support.fm(0x80 + offset, 0x0F)
            stream += Support.fm(0x90 + offset, 0x0A)
        }
        stream += Support.fm(0xB0, 4 | 5 << 3) + Support.fm(0xB4, 0xC0)
        stream += Support.fm(0xA4, 0x24) + Support.fm(0xA0, 0x3B) + Support.wait(1000)
        let m = Support.machine(Support.vgm(stream: stream))
        defer { cgenesis_free(m) }
        _ = Support.render(m, frames: 100)
        var fm = cgenesis_fm()
        #expect(cgenesis_fm_get(m, 0, &fm) == 1)
        #expect(fm.chip == 0 && fm.algorithm == 4 && fm.feedback == 5)
        #expect(fm.instrument == 0 && fm.operators == 4)
        let ops = Self.ops(fm)
        #expect(ops.map(\.multiple) == [1, 2, 3, 4])
        #expect(ops.map(\.total_level) == [0x11, 0x22, 0x33, 0x44])
        #expect(ops.allSatisfy { $0.attack_rate == 31 && $0.am == 1 && $0.ssg_eg == 0x0A })
        #expect(ops.allSatisfy { $0.release_rate == 15 && $0.phase == 4 && $0.envelope_level == 1023 })
        #expect(abs(ops[0].frequency_hz - 440.1) < 1)
        #expect(abs(ops[3].frequency_hz / ops[0].frequency_hz - 4) < 0.05)
    }

    @Test func onlyFmVoicesHaveAPatch() {
        let m = Support.machine(Support.vgm(stream: Support.fmTone() + Support.psgTone() + Support.wait(1000)))
        defer { cgenesis_free(m) }
        _ = Support.render(m, frames: 100)
        var fm = cgenesis_fm()
        #expect(cgenesis_fm_get(m, 0, &fm) == 1)
        #expect(fm.algorithm == 7 && fm.op.0.phase == 2 && fm.op.0.envelope_level == 0)
        var untouched = cgenesis_fm()
        untouched.chip = 99
        for voice: Int32 in [6, 9, 10, -1, 1000] {
            #expect(cgenesis_fm_get(m, voice, &untouched) == 0)
        }
        #expect(untouched.chip == 99)
        #expect(cgenesis_fm_get(m, 0, nil) == 0)
        #expect(cgenesis_fm_get(nil, 0, &untouched) == 0)
    }

    @Test func aYm2413PatchShowsTheBuiltInInstrumentAndTheUserTone() {
        // Instrument 3 at volume 4 on channel 1, the user tone on channel 2.
        var stream = Self.opll(0x30, 0x34) + Self.opll(0x10, 0x22) + Self.opll(0x20, 0x19)
        let user: [UInt8] = [0xE5, 0x5A, 0xC9, 0x1D, 0x73, 0xA4, 0x5B, 0x2C]
        for (i, v) in user.enumerated() { stream += Self.opll(UInt8(i), v) }
        stream += Self.opll(0x31, 0x07) + Self.opll(0x11, 0x22) + Self.opll(0x21, 0x09)
        stream += Support.wait(1000)
        let m = Support.machine(Self.opllVgm(stream: stream))
        defer { cgenesis_free(m) }
        #expect(cgenesis_voice_count(m) == 9)
        _ = Support.render(m, frames: 100)
        var fm = cgenesis_fm()
        #expect(cgenesis_fm_get(m, 0, &fm) == 1)
        #expect(fm.chip == 1 && fm.algorithm == 0 && fm.feedback == 0)
        #expect(fm.instrument == 3 && fm.operators == 2)
        var ops = Self.ops(fm)
        #expect(ops[0].multiple == 3 && ops[0].key_scale == 1 && ops[0].total_level == 0x19)
        #expect(ops[0].attack_rate == 15 && ops[0].decay_rate == 2)
        #expect(ops[0].sustain_level == 1 && ops[0].release_rate == 1)
        #expect(ops[1].multiple == 1 && ops[1].key_scale == 0 && ops[1].total_level == 16)
        #expect(ops[1].attack_rate == 12 && ops[1].decay_rate == 4)
        #expect(ops[1].sustain_level == 2 && ops[1].release_rate == 3)
        #expect(abs(ops[1].frequency_hz - 440) < 0.5 && abs(ops[0].frequency_hz - 1320) < 1.5)
        #expect(ops[1].phase < 4 && ops[1].envelope_level < 127)
        #expect(ops[2].multiple == 0 && ops[3].frequency_hz == 0)

        #expect(cgenesis_fm_get(m, 1, &fm) == 1)
        #expect(fm.instrument == 0 && fm.feedback == 5)
        ops = Self.ops(fm)
        #expect(ops[0].multiple == 5 && ops[0].total_level == 9 && ops[0].waveform == 1)
        #expect(ops[0].am == 1 && ops[0].vibrato == 1 && ops[0].sustained == 1)
        #expect(ops[1].multiple == 10 && ops[1].total_level == 28 && ops[1].waveform == 1)
        #expect(ops[1].am == 0 && ops[1].vibrato == 1 && ops[1].sustained == 0)
        #expect(ops[1].key_scale == 1 && ops[1].release_rate == 12 && ops[1].sustain_level == 2)

        let regs = Self.block(m, 0)
        #expect(regs.count == 64)
        #expect(Array(regs[0..<8]) == user)
        #expect(regs[0x30] == 0x34 && regs[0x20] == 0x19 && regs[0x31] == 0x07)
    }

    final class Probe {
        var machine: OpaquePointer?
        var answers: [Int] = []
    }

    @Test func theCallsDoNothingInsideAnObserverCallback() {
        let m = Support.machine(Support.vgm(stream: Support.fmTone() + Support.wait(1000)))
        defer { cgenesis_free(m) }
        let probe = Probe()
        probe.machine = m
        var observer = cgenesis_observer(
            context: Unmanaged.passUnretained(probe).toOpaque(),
            register_write: { context, _, _, _, _ in
                let probe = Unmanaged<Probe>.fromOpaque(context!).takeUnretainedValue()
                let m = probe.machine
                var bytes = [UInt8](repeating: 0xEE, count: 600)
                var fm = cgenesis_fm()
                probe.answers = [
                    Int(cgenesis_register_block_count(m)),
                    cgenesis_register_block_name(m, 0) == nil ? 1 : 0,
                    Int(cgenesis_register_block_base(m, 0)),
                    cgenesis_register_block_size(m, 0),
                    cgenesis_register_block_get(m, 0, &bytes, 600),
                    Int(cgenesis_fm_get(m, 0, &fm)),
                ]
            })
        cgenesis_set_observer(m, &observer)
        _ = Support.render(m, frames: 100)
        #expect(probe.answers == [0, 1, 0, 0, 0, 0])
        cgenesis_set_observer(m, nil)
        #expect(cgenesis_register_block_count(m) == 2)
    }

    @Test func readingDoesNotChangeTheSound() {
        let stream = Support.fmTone() + Support.psgTone() + Support.wait(30_000)
        let file = Support.vgm(stream: stream)
        let quiet = Support.machine(file)
        let curious = Support.machine(file)
        defer { cgenesis_free(quiet); cgenesis_free(curious) }
        var bytes = [UInt8](repeating: 0, count: 600)
        var fm = cgenesis_fm()
        for _ in 0..<30 {
            #expect(Support.render(quiet, frames: 531) == Support.render(curious, frames: 531))
            for block: Int32 in 0..<3 {
                _ = cgenesis_register_block_get(curious, block, &bytes, 600)
                _ = cgenesis_register_block_name(curious, block)
            }
            for voice: Int32 in 0..<11 { _ = cgenesis_fm_get(curious, voice, &fm) }
        }
        #expect(cgenesis_samples(quiet) == cgenesis_samples(curious))
    }
}
