import Testing
import CGba

/// Snapshots, the channel mask and the end-of-song signals, through the C interface.
struct HooksTests {
    static let buffer: [UInt8] = (0..<4096).map { UInt8(truncatingIfNeeded: ($0 * $0) >> 3) }
    static let rom = Support.rom(words: MachineTests.fifoProgram) + buffer

    @Test func aRestoredSnapshotReplaysTheSameSound() throws {
        let m = try #require(Support.machine(rom: Self.rom, rate: 48000))
        defer { cgba_free(m) }
        var warmup = [Int16](repeating: 0, count: 2 * 3000)
        cgba_render_s16(m, &warmup, 3000)
        let snapshot = try #require(cgba_snapshot_take(m))
        defer { cgba_snapshot_free(snapshot) }
        #expect(cgba_snapshot_size(snapshot) > 600_000)
        var first = [Int16](repeating: 0, count: 2 * 9600)
        cgba_render_s16(m, &first, 9600)
        #expect(cgba_snapshot_restore(m, snapshot) == 0)
        var again = first
        cgba_render_s16(m, &again, 9600)
        #expect(first == again)
        #expect(first.contains { $0 != first[0] }, "the FIFO output is audible")

        let other = try #require(Support.machine(rom: Self.rom, rate: 48000))
        defer { cgba_free(other) }
        #expect(cgba_snapshot_restore(other, snapshot) == 0)
        var third = first
        cgba_render_s16(other, &third, 9600)
        #expect(first == third)
    }

    @Test func aSnapshotRefusesAMachineThatDiffers() throws {
        let m = try #require(Support.machine(rom: Self.rom, rate: 48000))
        let otherRate = try #require(Support.machine(rom: Self.rom, rate: 44100))
        let otherImage = try #require(Support.machine(rom: Self.rom + [1, 2, 3, 4], rate: 48000))
        defer { cgba_free(m); cgba_free(otherRate); cgba_free(otherImage) }
        let snapshot = try #require(cgba_snapshot_take(m))
        defer { cgba_snapshot_free(snapshot) }
        #expect(cgba_snapshot_restore(otherRate, snapshot) == 1)
        #expect(cgba_snapshot_restore(otherImage, snapshot) == 2)
    }

    @Test func theChannelMaskSilencesTheMixAndSurvivesAReset() throws {
        let m = try #require(Support.machine(rom: Self.rom, rate: 48000))
        defer { cgba_free(m) }
        #expect(cgba_channel_mask(m) == 0x3F)
        cgba_set_channel_mask(m, 0)
        var out = [Int16](repeating: 0, count: 2 * 9600)
        cgba_render_s16(m, &out, 9600)
        let settled = out[800...]
        #expect(settled.allSatisfy { $0 == settled.first }, "a muted mix is flat")
        cgba_reset(m)
        #expect(cgba_channel_mask(m) == 0)
        var levels = cgba_channel_levels()
        cgba_render_s16(m, &out, 9600)
        cgba_channel_levels_get(m, &levels)
        #expect(levels.fifo.0 != 0 || levels.fifo.1 != 0, "a muted channel still shows its level")
    }

    @Test func theEndOfASongIsTold() throws {
        let parked = try #require(Support.machine(rom: Support.rom(words: [0xEAFF_FFFE])))
        let busy = try #require(Support.machine(rom: Support.rom(words: [0xE280_0001, 0xEAFF_FFFD])))
        let halted = try #require(Support.machine(rom: Support.rom(words: [0xEF02_0000, 0xEAFF_FFFE])))
        defer { cgba_free(parked); cgba_free(busy); cgba_free(halted) }
        cgba_run_cycles(parked, 100)
        cgba_run_cycles(busy, 100)
        cgba_run_cycles(halted, 2000)
        #expect(cgba_end_state_get(parked) == CGBA_PARKED)
        #expect(cgba_end_state_get(busy) == CGBA_RUNNING)
        #expect(cgba_end_state_get(halted) == CGBA_HALTED_WITHOUT_WAKE)
        #expect(cgba_quiet_cycles(parked) > 0)
    }

    @Test func theWarningListStaysAtOneAddress() throws {
        // An undefined instruction makes the machine report once.
        let m = try #require(Support.machine(rom: Support.rom(words: [0xE600_0010, 0xEAFF_FFFE])))
        defer { cgba_free(m) }
        let before = try #require(cgba_warnings(m))
        #expect(String(cString: before).isEmpty)
        cgba_run_cycles(m, 100)
        #expect(cgba_warnings(m) == before, "the list does not move when a warning arrives")
        #expect(String(cString: before).contains("undefined"))
        let snapshot = try #require(cgba_snapshot_take(m))
        defer { cgba_snapshot_free(snapshot) }
        cgba_reset(m)
        #expect(cgba_warnings(m) == before, "a reset keeps the address")
        #expect(String(cString: before).isEmpty)
        #expect(cgba_snapshot_restore(m, snapshot) == 0)
        #expect(cgba_warnings(m) == before, "a restore keeps the address")
        #expect(String(cString: before).contains("undefined"))
    }

    final class Reentrant {
        var machine: OpaquePointer?
        var calls = 0
        var snapshotStatus: Int32 = -1
    }

    @Test func aCallbackCannotDisturbTheMachineThatCallsIt() throws {
        let buffer: [UInt8] = (0..<256).map { UInt8(truncatingIfNeeded: $0 * 7 + 3) }
        let m = try #require(Support.machine(rom: Support.rom(words: MachineTests.fifoProgram) + buffer))
        defer { cgba_free(m) }
        let box = Reentrant()
        box.machine = m
        var observer = cgba_observer(
            context: Unmanaged.passUnretained(box).toOpaque(),
            sound_write: { ctx, _, _, _ in
                let box = Unmanaged<Reentrant>.fromOpaque(ctx!).takeUnretainedValue()
                box.calls += 1
                // Each of these would alias the machine that is rendering. They do nothing.
                cgba_set_observer(box.machine, nil)
                cgba_run_cycles(box.machine, 1000)
                cgba_reset(box.machine)
                cgba_set_channel_mask(box.machine, 0)
                cgba_free(box.machine)
                if let snapshot = cgba_snapshot_take(box.machine) { cgba_snapshot_free(snapshot) }
                // The one call that works there.
                _ = cgba_cycles(box.machine)
            },
            fifo_sample: nil)
        cgba_set_observer(m, &observer)
        cgba_run_cycles(m, 512 * 300)
        #expect(box.calls > 64, "the observer stayed installed")
        #expect(cgba_channel_mask(m) == 0x3F)
        #expect(cgba_cycles(m) >= 512 * 300)
        cgba_set_observer(m, nil)
    }
}
