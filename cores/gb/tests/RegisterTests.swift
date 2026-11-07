import Testing
import CGb

/// The register blocks through the C interface.
@Suite("Register blocks")
struct RegisterTests {
    final class Log {
        var machine: OpaquePointer?
        /// What the block calls answered from inside a callback.
        var inside: [Int] = []
        var names: [UnsafePointer<CChar>?] = []
    }

    static func block(_ m: OpaquePointer, _ index: Int32) -> [UInt8] {
        var out = [UInt8](repeating: 0xEE, count: cgb_register_block_size(m, index))
        #expect(cgb_register_block_get(m, index, &out, out.count) == out.count)
        return out
    }

    /// The boot values, before the program stores anything.
    static var boot: [UInt8] {
        var b = [UInt8](repeating: 0, count: 23)
        b[0x01] = 0x80; b[0x02] = 0xF3; b[0x14] = 0x77; b[0x15] = 0xF3; b[0x16] = 0x80
        return b
    }

    @Test func theBlocksHaveNamesSizesAndBases() throws {
        let m = try #require(Support.machine(init: Support.note))
        defer { cgb_free(m) }
        #expect(cgb_register_block_count(m) == 2)
        #expect(String(cString: cgb_register_block_name(m, 0)) == "APU")
        #expect(String(cString: cgb_register_block_name(m, 1)) == "Wave RAM")
        #expect(cgb_register_block_name(m, 0) == cgb_register_block_name(m, 0), "the same pointer each call")
        #expect(cgb_register_block_name(m, 2) == nil)
        #expect(cgb_register_block_name(m, -1) == nil)
        #expect(cgb_register_block_size(m, 0) == 23 && cgb_register_block_size(m, 1) == 16)
        #expect(cgb_register_block_size(m, 2) == 0 && cgb_register_block_size(m, -1) == 0)
        #expect(cgb_register_block_base(m, 0) == 0xFF10 && cgb_register_block_base(m, 1) == 0xFF30)
        #expect(cgb_register_block_base(m, 2) == 0 && cgb_register_block_base(m, -1) == 0)
    }

    @Test func aProgramReadsBackWhatItStored() throws {
        var ram: [UInt8] = []
        var code = Support.note
        for i in 0..<16 {
            ram.append(UInt8((i * 16) ^ 0x3C))
            code += Support.store(UInt8(0x30 + i), ram[i])
        }
        let m = try #require(Support.machine(init: code))
        defer { cgb_free(m) }
        #expect(Self.block(m, 0) == Self.boot, "before the program runs")
        _ = Support.render(m, frames: 2000)
        var apu = [UInt8](repeating: 0, count: 23)
        apu[0x14] = 0x77; apu[0x15] = 0xFF; apu[0x16] = 0x80
        apu[0x06] = 0x80; apu[0x07] = 0xF0; apu[0x08] = 0x00; apu[0x09] = 0x87
        apu[0x01] = 0x80; apu[0x02] = 0xF3
        #expect(Self.block(m, 0) == apu)
        #expect(Self.block(m, 1) == ram)
        #expect(cgb_peek(m, 0xFF19) == 0xBF, "the register reads back with its write-only bits set")
    }

    @Test func shortBuffersAndInvalidArgumentsGetNothingOrAPrefix() throws {
        let m = try #require(Support.machine(init: Support.note))
        defer { cgb_free(m) }
        _ = Support.render(m, frames: 2000)
        let all = Self.block(m, 0)
        var short = [UInt8](repeating: 0xEE, count: 5)
        #expect(cgb_register_block_get(m, 0, &short, 5) == 5)
        #expect(short == Array(all[0..<5]))
        var big = [UInt8](repeating: 0xEE, count: 40)
        #expect(cgb_register_block_get(m, 0, &big, 40) == 23)
        #expect(Array(big[0..<23]) == all && big[23...].allSatisfy { $0 == 0xEE })
        var out = [UInt8](repeating: 0xEE, count: 8)
        #expect(cgb_register_block_get(m, 0, &out, 0) == 0)
        #expect(cgb_register_block_get(m, 0, nil, 8) == 0)
        #expect(cgb_register_block_get(m, 2, &out, 8) == 0)
        #expect(cgb_register_block_get(m, -1, &out, 8) == 0)
        #expect(out.allSatisfy { $0 == 0xEE })
        #expect(cgb_register_block_count(nil) == 0)
        #expect(cgb_register_block_name(nil, 0) == nil)
        #expect(cgb_register_block_base(nil, 0) == 0)
        #expect(cgb_register_block_size(nil, 0) == 0)
        #expect(cgb_register_block_get(nil, 0, &out, 8) == 0)
    }

    @Test func nothingHereWorksFromInsideACallback() throws {
        let m = try #require(Support.machine(init: Support.note))
        defer { cgb_free(m) }
        let log = Log()
        log.machine = m
        var observer = cgb_observer(
            context: Unmanaged.passUnretained(log).toOpaque(),
            sound_write: { context, _, _ in
                let log = Unmanaged<Log>.fromOpaque(context!).takeUnretainedValue()
                var out = [UInt8](repeating: 0xEE, count: 23)
                log.inside.append(Int(cgb_register_block_count(log.machine)))
                log.inside.append(Int(cgb_register_block_base(log.machine, 0)))
                log.inside.append(cgb_register_block_size(log.machine, 0))
                log.inside.append(cgb_register_block_get(log.machine, 0, &out, 23))
                log.names.append(cgb_register_block_name(log.machine, 0))
            },
            play_call: nil)
        cgb_set_observer(m, &observer)
        _ = Support.render(m, frames: 1000)
        cgb_set_observer(m, nil)
        #expect(!log.inside.isEmpty)
        #expect(log.inside.allSatisfy { $0 == 0 })
        #expect(log.names.allSatisfy { $0 == nil })
        #expect(cgb_register_block_count(m) == 2, "outside the callback the calls answer again")
    }

    @Test func readingTheBlocksDoesNotChangeTheSound() throws {
        let play = Support.store(0x18, 0x10) + Support.store(0x19, 0x86)
        let a = try #require(Support.machine(init: Support.note, play: play))
        let b = try #require(Support.machine(init: Support.note, play: play))
        defer { cgb_free(a); cgb_free(b) }
        var plain: [Int16] = [], read: [Int16] = []
        for _ in 0..<20 {
            plain += Support.render(a, frames: 500)
            read += Support.render(b, frames: 500)
            for i in 0..<cgb_register_block_count(b) {
                _ = Self.block(b, i)
                _ = cgb_register_block_name(b, i)
                _ = cgb_register_block_base(b, i)
            }
        }
        #expect(plain == read)
    }

    @Test func aRestoreBringsTheBytesBack() throws {
        let m = try #require(Support.machine(init: Support.note, play: Support.store(0x18, 0x42)))
        defer { cgb_free(m) }
        let start = try #require(cgb_snapshot_take(m))
        defer { cgb_snapshot_free(start) }
        #expect(Self.block(m, 0) == Self.boot)
        _ = Support.render(m, frames: 4000)
        let ran = Self.block(m, 0)
        #expect(ran != Self.boot && ran[0x08] == 0x42)
        #expect(cgb_snapshot_restore(m, start) == 0)
        #expect(Self.block(m, 0) == Self.boot)
        _ = Support.render(m, frames: 4000)
        #expect(Self.block(m, 0) == ran)
        cgb_reset(m)
        #expect(Self.block(m, 0) == Self.boot)
    }
}
