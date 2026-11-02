import Testing
import CNes

/// The observer, snapshots, the channel mask and the bass cutoff, through the C interface.
struct HooksTests {
    final class Log {
        var machine: OpaquePointer?
        var writes: [(cycle: UInt64, address: UInt32, value: UInt32)] = []
        var calls: [(cycle: UInt64, frame: Int64)] = []
        /// What the machine answered to calls other than `cnes_cycles` from inside a callback.
        var inside: [UInt32] = []
    }

    static func observe(_ m: OpaquePointer, into log: Log) -> cnes_observer {
        log.machine = m
        return cnes_observer(
            context: Unmanaged.passUnretained(log).toOpaque(),
            sound_write: { context, address, value in
                let log = Unmanaged<Log>.fromOpaque(context!).takeUnretainedValue()
                log.writes.append((cnes_cycles(log.machine), address, value))
                // Everything but cnes_cycles does nothing in here.
                log.inside.append(cnes_register(log.machine, 0))
                log.inside.append(cnes_next_pc(log.machine))
                log.inside.append(cnes_peek(log.machine, 0x0200))
                log.inside.append(UInt32(cnes_frames(log.machine)))
                log.inside.append(UInt32(truncatingIfNeeded: cnes_quiet_cycles(log.machine)))
                log.inside.append(UInt32(truncatingIfNeeded: cnes_set_highpass_hz(log.machine, 200)))
                log.inside.append(UInt32(cnes_highpass_hz(log.machine)))
            },
            play_call: { context, frame in
                let log = Unmanaged<Log>.fromOpaque(context!).takeUnretainedValue()
                log.calls.append((cnes_cycles(log.machine), frame))
            })
    }

    @Test func theObserverSeesEveryStoreAndEveryCall() throws {
        let m = try #require(Support.machine(init: Support.store(0x4015, 0x0F), play: Support.store(0x4017, 0x40)))
        defer { cnes_free(m) }
        let log = Log()
        var observer = Self.observe(m, into: log)
        cnes_set_observer(m, &observer)
        cnes_run_cycles(m, UInt64(CNES_NTSC_CLOCK_HZ / 10))
        #expect(log.calls.first?.frame == -1)
        #expect(log.calls.dropFirst().first?.frame == 0)
        #expect(log.calls.dropFirst().first?.cycle == 29780, "16639 microseconds of NTSC cycles, rounded down")
        #expect(log.writes.first?.address == 0x4015)
        #expect(log.writes.first?.value == 0x0F)
        #expect(log.writes.dropFirst().first?.address == 0x4017)
        #expect(log.writes.count == 1 + log.calls.count - 1)
        #expect(log.inside.allSatisfy { $0 == 0 }, "calls other than cnes_cycles return neutral values")
        cnes_set_observer(m, nil)
        let seen = log.writes.count
        cnes_run_cycles(m, UInt64(CNES_NTSC_CLOCK_HZ / 10))
        #expect(log.writes.count == seen)
    }

    @Test func aStoreOutsideTheSoundRegistersIsNotReported() throws {
        let m = try #require(Support.machine(init: Support.store(0x0200, 1) + Support.store(0x4018, 1) + Support.store(0x4000, 2)))
        defer { cnes_free(m) }
        let log = Log()
        var observer = Self.observe(m, into: log)
        cnes_set_observer(m, &observer)
        cnes_run_cycles(m, 1000)
        #expect(log.writes.map(\.address) == [0x4000])
    }

    @Test func aRestoredSnapshotReplaysTheSameSound() throws {
        let play = Support.store(0x4002, 0x10) + Support.store(0x4003, 0x0A)
        let m = try #require(Support.machine(init: Support.note, play: play))
        defer { cnes_free(m) }
        _ = Support.render(m, frames: 3000)
        let snapshot = try #require(cnes_snapshot_take(m))
        defer { cnes_snapshot_free(snapshot) }
        #expect(cnes_snapshot_size(snapshot) > 10_000)
        let first = Support.render(m, frames: 9600)
        #expect(cnes_snapshot_restore(m, snapshot) == 0)
        #expect(Support.render(m, frames: 9600) == first)

        let other = try #require(Support.machine(init: Support.note, play: play))
        defer { cnes_free(other) }
        #expect(cnes_snapshot_restore(other, snapshot) == 0)
        #expect(Support.render(other, frames: 9600) == first)
    }

    @Test func aSnapshotRefusesAMachineThatDiffers() throws {
        let m = try #require(Support.machine(init: Support.note))
        let otherRate = try #require(Support.machine(init: Support.note, rate: 44100))
        let otherFile = try #require(Support.machine(init: Support.power))
        let otherRegion = try #require(Support.machine(init: Support.note, region: CNES_PAL))
        defer { cnes_free(m); cnes_free(otherRate); cnes_free(otherFile); cnes_free(otherRegion) }
        let snapshot = try #require(cnes_snapshot_take(m))
        defer { cnes_snapshot_free(snapshot) }
        #expect(cnes_snapshot_restore(otherRate, snapshot) == 1)
        #expect(cnes_snapshot_restore(otherFile, snapshot) == 2)
        #expect(cnes_snapshot_restore(otherRegion, snapshot) == 2)
        #expect(cnes_snapshot_restore(m, nil) == 2)
        #expect(cnes_snapshot_size(nil) == 0)
        cnes_snapshot_free(nil)
    }

    @Test func theChannelMaskSilencesAChannelAndSurvivesAReset() throws {
        let m = try #require(Support.machine(init: Support.note))
        defer { cnes_free(m) }
        #expect(cnes_channel_mask(m) == 0x1F)
        cnes_set_channel_mask(m, 0b11110)
        _ = Support.render(m, frames: 48000)
        let tail = Support.render(m, frames: 9600)
        #expect(tail.allSatisfy { abs(Int($0)) < 50 })
        cnes_reset(m)
        #expect(cnes_channel_mask(m) == 0b11110)
        var levels = [Int32](repeating: 0, count: 5)
        var loudest: Int32 = 0
        for _ in 0..<64 {
            cnes_run_cycles(m, 4001)
            cnes_channel_levels_get(m, &levels)
            loudest = max(loudest, levels[0])
        }
        #expect(loudest == 15, "a muted channel still shows its level")
        cnes_set_channel_mask(m, 0xFFFF_FFFF)
        #expect(cnes_channel_mask(m) == 0x1F, "only five bits count")
    }

    @Test func theBassCutoffStartsAtTheConsoleAndAcceptsOnlyItsRange() throws {
        let m = try #require(Support.machine(init: Support.note))
        defer { cnes_free(m) }
        #expect(cnes_highpass_hz(m) == CNES_HIGHPASS_CONSOLE_HZ)
        #expect(cnes_set_highpass_hz(m, 300) == 1)
        #expect(cnes_highpass_hz(m) == 300)
        for bad in [0, CNES_HIGHPASS_MIN_HZ - 0.5, CNES_HIGHPASS_MAX_HZ + 0.5, -5, .nan, .infinity] {
            #expect(cnes_set_highpass_hz(m, bad) == 0, "\(bad)")
            #expect(cnes_highpass_hz(m) == 300)
        }
        #expect(cnes_set_highpass_hz(m, CNES_HIGHPASS_MIN_HZ) == 1)
        #expect(cnes_set_highpass_hz(m, CNES_HIGHPASS_MAX_HZ) == 1)
        cnes_use_console_highpass(m)
        #expect(cnes_highpass_hz(m) == CNES_HIGHPASS_CONSOLE_HZ)
    }

    @Test func theBassCutoffChangesTheSoundDuringPlayAndSurvivesAReset() throws {
        let plain = try #require(Support.machine(init: Support.note))
        let thin = try #require(Support.machine(init: Support.note))
        defer { cnes_free(plain); cnes_free(thin) }
        _ = Support.render(plain, frames: 4800)
        _ = Support.render(thin, frames: 4800)
        #expect(cnes_set_highpass_hz(thin, 1000) == 1)
        let a = Support.render(plain, frames: 9600)
        let b = Support.render(thin, frames: 9600)
        #expect(a != b)
        cnes_reset(thin)
        #expect(cnes_highpass_hz(thin) == 1000)
        cnes_use_console_highpass(thin)
        cnes_reset(thin)
        cnes_reset(plain)
        #expect(Support.render(thin, frames: 9600) == Support.render(plain, frames: 9600))
    }

    @Test func theChannelLevelsAreFiveNumbersInTheirRanges() throws {
        let m = try #require(Support.machine(init: Support.note))
        defer { cnes_free(m) }
        var seen = Set<Int32>()
        var levels = [Int32](repeating: -1, count: 5)
        for _ in 0..<64 {
            cnes_run_cycles(m, 4001)
            cnes_channel_levels_get(m, &levels)
            seen.insert(levels[0])
            #expect((0...15).contains(levels[0]) && (0...15).contains(levels[1]))
            #expect((0...15).contains(levels[2]) && (0...15).contains(levels[3]))
            #expect((0...127).contains(levels[4]))
        }
        #expect(seen.contains(15) && seen.contains(0), "the pulse alternates")
        #expect(levels[1] == 0 && levels[3] == 0 && levels[4] == 0, "the other channels are idle")
        cnes_channel_levels_get(m, nil)
    }

    @Test func eachChannelShowsItsOwnLevel() throws {
        let setup = Support.store(0x4015, 0x0F)
            + Support.store(0x4004, 0xBF) + Support.store(0x4006, 0xFF) + Support.store(0x4007, 0x08)
            + Support.store(0x4008, 0xFF) + Support.store(0x400A, 0x40) + Support.store(0x400B, 0x08)
            + Support.store(0x400C, 0x3F) + Support.store(0x400E, 0x04) + Support.store(0x400F, 0x08)
            + Support.store(0x4011, 0x40)
        let m = try #require(Support.machine(init: setup))
        defer { cnes_free(m) }
        var loudest = [Int32](repeating: 0, count: 5)
        var levels = [Int32](repeating: 0, count: 5)
        for _ in 0..<400 {
            cnes_run_cycles(m, 997)
            cnes_channel_levels_get(m, &levels)
            for i in 0..<5 { loudest[i] = max(loudest[i], levels[i]) }
        }
        #expect(loudest[0] == 0, "pulse 1 was never started")
        #expect(loudest[1] == 15)
        #expect(loudest[2] > 8 && loudest[2] <= 15)
        #expect(loudest[3] == 15)
        #expect(levels[4] == 0x40, "a store to the delta channel's level register shows at once")
    }
}
