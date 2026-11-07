import Testing
import CNes

/// The register blocks, through the C interface.
@Suite("Register blocks")
struct RegisterBlockTests {
    static func names(_ m: OpaquePointer) -> [String] {
        (0..<Int32(cnes_register_block_count(m))).map { String(cString: cnes_register_block_name(m, $0)) }
    }

    static func block(_ m: OpaquePointer, _ name: String) -> [UInt8] {
        guard let index = names(m).firstIndex(of: name) else { return [] }
        var out = [UInt8](repeating: 0xEE, count: cnes_register_block_size(m, Int32(index)))
        #expect(cnes_register_block_get(m, Int32(index), &out, out.count) == out.count)
        return out
    }

    /// Stores `value` at each address.
    static func stores(_ list: [(UInt16, UInt8)]) -> [UInt8] {
        list.flatMap { Support.store($0.0, $0.1) }
    }

    @Test func aFileWithoutChipsHasTheAPUBlock() throws {
        let m = try #require(Support.machine(init: Self.stores([(0x4000, 0x55), (0x4003, 0x08), (0x4016, 0x01)])))
        defer { cnes_free(m) }
        _ = Support.render(m, frames: 2000)
        #expect(cnes_register_block_count(m) == 1)
        #expect(String(cString: cnes_register_block_name(m, 0)) == "2A03 APU")
        #expect(cnes_register_block_name(m, 0) == cnes_register_block_name(m, 0), "the same pointer on every call")
        #expect(cnes_register_block_base(m, 0) == 0x4000)
        #expect(cnes_register_block_size(m, 0) == 24)
        var want = [UInt8](repeating: 0, count: 24)
        want[0] = 0x55
        want[3] = 0x08
        want[0x15] = 0x0F
        want[0x16] = 0x01
        want[0x17] = 0x40
        #expect(Self.block(m, "2A03 APU") == want)
    }

    @Test func theBlocksFollowTheChipsInAFixedOrder() throws {
        let m = try #require(Support.machine(init: [], chips: 0x7F))
        defer { cnes_free(m) }
        #expect(Self.names(m) == ["2A03 APU", "VT02", "VRC6", "5B", "N163", "FDS", "MMC5", "VRC7"])
        let sizes = (0..<8).map { cnes_register_block_size(m, Int32($0)) }
        #expect(sizes == [24, 22, 12, 16, 128, 88, 22, 54])
        let bases = (0..<8).map { cnes_register_block_base(m, Int32($0)) }
        #expect(bases == [0x4000, 0x4020, 0x9000, 0, 0, 0x4040, 0x5000, 0])

        let two = try #require(Support.machine(init: [], chips: 4 | 2))
        defer { cnes_free(two) }
        #expect(Self.names(two) == ["2A03 APU", "FDS", "VRC7"])
        #expect(cnes_register_block_base(two, 1) == 0x4040)
    }

    @Test func theChipBlocksHoldWhatTheProgramWrote() throws {
        // The VRC6 at 0x9000 to 0xB003, the FDS's wave RAM and registers, the
        // MMC5 and the VRC7 through its ports.
        var setup = Self.stores([(0x9000, 0x11), (0x9004, 0x12), (0xA001, 0x21), (0xB003, 0x31)])
        setup += Self.stores([(0x4089, 0x80), (0x4040, 0x3F), (0x4041, 0x45), (0x4080, 0x9C)])
        setup += Self.stores([(0x5000, 0x44), (0x5015, 0x03), (0x5205, 0xFF)])
        let m = try #require(Support.machine(init: setup, chips: 1 | 4 | 8))
        defer { cnes_free(m) }
        _ = Support.render(m, frames: 2000)
        let vrc6 = Self.block(m, "VRC6")
        #expect(vrc6.count == 12)
        #expect(vrc6[0] == 0x12 && vrc6[5] == 0x21 && vrc6[11] == 0x31 && vrc6[1] == 0)
        let fds = Self.block(m, "FDS")
        #expect(fds.count == 88)
        #expect(fds[0] == 0x3F && fds[1] == 0x05, "the wave RAM keeps six bits")
        #expect(fds[64] == 0x9C && fds[73] == 0x80)
        let mmc5 = Self.block(m, "MMC5")
        #expect(mmc5.count == 22 && mmc5[0] == 0x44 && mmc5[0x15] == 0x03)

        let vrc7 = try #require(Support.machine(init: Self.stores([
            (0x9010, 0x10), (0x9030, 0xAB), (0x9010, 0x08), (0x9030, 0x77), (0x9010, 0x35), (0x9030, 0x1F),
        ]), chips: 2))
        defer { cnes_free(vrc7) }
        _ = Support.render(vrc7, frames: 2000)
        let regs = Self.block(vrc7, "VRC7")
        #expect(regs.count == 54)
        #expect(regs[0x10] == 0xAB && regs[0x35] == 0x1F && regs[0x08] == 0)

        let n163 = try #require(Support.machine(init: Self.stores([
            (0xF800, 0x80), (0x4800, 0x21), (0x4800, 0x43),
        ]), chips: 16))
        defer { cnes_free(n163) }
        _ = Support.render(n163, frames: 2000)
        let ram = Self.block(n163, "N163")
        #expect(ram.count == 128 && ram[0] == 0x21 && ram[1] == 0x43)

        let s5b = try #require(Support.machine(init: Self.stores([
            (0xC000, 7), (0xE000, 0x38), (0xC000, 0x0F), (0xE000, 0x01),
        ]), chips: 32))
        defer { cnes_free(s5b) }
        _ = Support.render(s5b, frames: 2000)
        let file = Self.block(s5b, "5B")
        #expect(file.count == 16 && file[7] == 0x38 && file[15] == 0x01)
    }

    @Test func invalidArgumentsGiveNothing() throws {
        let m = try #require(Support.machine(init: Self.stores([(0x4000, 0x55)])))
        defer { cnes_free(m) }
        _ = Support.render(m, frames: 2000)
        var out = [UInt8](repeating: 0xEE, count: 40)
        #expect(cnes_register_block_name(m, 1) == nil)
        #expect(cnes_register_block_name(m, -1) == nil)
        #expect(cnes_register_block_size(m, 1) == 0 && cnes_register_block_size(m, -1) == 0)
        #expect(cnes_register_block_base(m, 1) == 0 && cnes_register_block_base(m, -1) == 0)
        #expect(cnes_register_block_get(m, 1, &out, out.count) == 0)
        #expect(cnes_register_block_get(m, -1, &out, out.count) == 0)
        #expect(cnes_register_block_get(m, 0, &out, 0) == 0)
        #expect(cnes_register_block_get(m, 0, nil, 24) == 0)
        #expect(out == [UInt8](repeating: 0xEE, count: 40))
        #expect(cnes_register_block_get(m, 0, &out, 3) == 3)
        #expect(Array(out[0..<4]) == [0x55, 0, 0, 0xEE])
        #expect(cnes_register_block_get(m, 0, &out, out.count) == 24)
        #expect(out[24] == 0xEE)
        #expect(cnes_register_block_count(nil) == 0)
        #expect(cnes_register_block_name(nil, 0) == nil)
        #expect(cnes_register_block_size(nil, 0) == 0)
        #expect(cnes_register_block_base(nil, 0) == 0)
        #expect(cnes_register_block_get(nil, 0, &out, out.count) == 0)
    }

    @Test func readingTheBlocksDoesNotChangeTheSound() throws {
        let play = Self.stores([(0x4002, 0x10), (0x4003, 0x0A)])
        let plain = try #require(Support.machine(init: Support.note, play: play))
        let looked = try #require(Support.machine(init: Support.note, play: play))
        defer { cnes_free(plain); cnes_free(looked) }
        var buffer = [UInt8](repeating: 0, count: 128)
        for _ in 0..<8 {
            #expect(Support.render(plain, frames: 700) == Support.render(looked, frames: 700))
            _ = cnes_register_block_get(looked, 0, &buffer, buffer.count)
            _ = cnes_register_block_name(looked, 0)
        }
    }

    @Test func aRestoredSnapshotRestoresTheBlocks() throws {
        let play = Self.stores([(0x4002, 0x10), (0x4003, 0x0A)]) + [0xE6, 0x00, 0xA5, 0x00, 0x8D, 0x22, 0x40]
        let m = try #require(Support.machine(init: Support.note, play: play, chips: 64))
        defer { cnes_free(m) }
        _ = Support.render(m, frames: 3000)
        let snapshot = try #require(cnes_snapshot_take(m))
        defer { cnes_snapshot_free(snapshot) }
        let before = [Self.block(m, "2A03 APU"), Self.block(m, "VT02")]
        _ = Support.render(m, frames: 9600)
        #expect([Self.block(m, "2A03 APU"), Self.block(m, "VT02")] != before)
        #expect(cnes_snapshot_restore(m, snapshot) == 0)
        #expect([Self.block(m, "2A03 APU"), Self.block(m, "VT02")] == before)
    }

    @Test func theCallsReturnNothingInsideAnObserverCallback() throws {
        final class Seen { var machine: OpaquePointer?; var values: [UInt32] = [] }
        let seen = Seen()
        let m = try #require(Support.machine(init: Self.stores([(0x4000, 0x55), (0x4001, 0x66)])))
        defer { cnes_free(m) }
        seen.machine = m
        var out = [UInt8](repeating: 0xEE, count: 24)
        let observer = cnes_observer(
            context: Unmanaged.passUnretained(seen).toOpaque(),
            sound_write: { context, _, _ in
                let s = Unmanaged<Seen>.fromOpaque(context!).takeUnretainedValue()
                var buffer = [UInt8](repeating: 0xEE, count: 24)
                s.values.append(UInt32(cnes_register_block_count(s.machine)))
                s.values.append(UInt32(cnes_register_block_size(s.machine, 0)))
                s.values.append(cnes_register_block_base(s.machine, 0))
                s.values.append(cnes_register_block_name(s.machine, 0) == nil ? 0 : 1)
                s.values.append(UInt32(cnes_register_block_get(s.machine, 0, &buffer, buffer.count)))
                s.values.append(buffer[0] == 0xEE ? 0 : 1)
            },
            play_call: nil)
        withExtendedLifetime(seen) {
            var o = observer
            cnes_set_observer(m, &o)
            _ = Support.render(m, frames: 500)
            cnes_set_observer(m, nil)
        }
        #expect(!seen.values.isEmpty)
        #expect(seen.values.allSatisfy { $0 == 0 }, "nothing is returned or written inside a callback")
        _ = cnes_register_block_get(m, 0, &out, out.count)
        #expect(out[0] == 0x55)
    }
}
