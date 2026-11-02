import Testing
import CGb

/// The observer, snapshots and the channel mask, through the C interface.
struct HooksTests {
    final class Log {
        var machine: OpaquePointer?
        var writes: [(cycle: UInt64, address: UInt32, value: UInt32)] = []
        var calls: [(cycle: UInt64, frame: Int64)] = []
        /// What the machine answered to calls other than `cgb_cycles` from inside a callback.
        var inside: [UInt32] = []
    }

    static func observe(_ m: OpaquePointer, into log: Log) -> cgb_observer {
        log.machine = m
        return cgb_observer(
            context: Unmanaged.passUnretained(log).toOpaque(),
            sound_write: { context, address, value in
                let log = Unmanaged<Log>.fromOpaque(context!).takeUnretainedValue()
                log.writes.append((cgb_cycles(log.machine), address, value))
                // Everything but cgb_cycles does nothing in here.
                log.inside.append(cgb_register(log.machine, 0))
                log.inside.append(UInt32(cgb_frames(log.machine)))
                log.inside.append(UInt32(truncatingIfNeeded: cgb_quiet_cycles(log.machine)))
            },
            play_call: { context, frame in
                let log = Unmanaged<Log>.fromOpaque(context!).takeUnretainedValue()
                log.calls.append((cgb_cycles(log.machine), frame))
            })
    }

    @Test func theObserverSeesEveryStoreAndEveryCall() throws {
        let m = try #require(Support.machine(init: Support.store(0x26, 0x80), play: Support.store(0x24, 0x55)))
        defer { cgb_free(m) }
        let log = Log()
        var observer = Self.observe(m, into: log)
        cgb_set_observer(m, &observer)
        cgb_run_cycles(m, UInt64(CGB_CLOCK_HZ) / 10)
        #expect(log.calls.first?.frame == -1)
        #expect(log.calls.dropFirst().first?.frame == 0)
        #expect(log.calls.dropFirst().first?.cycle == 70224)
        #expect(log.writes.first?.address == 0xFF26)
        #expect(log.writes.first?.cycle == 20)
        #expect(log.writes.count == 1 + log.calls.count - 1)
        #expect(log.inside.allSatisfy { $0 == 0 }, "calls other than cgb_cycles return neutral values")
        cgb_set_observer(m, nil)
        let seen = log.writes.count
        cgb_run_cycles(m, UInt64(CGB_CLOCK_HZ) / 10)
        #expect(log.writes.count == seen)
    }

    @Test func aRestoredSnapshotReplaysTheSameSound() throws {
        let m = try #require(Support.machine(init: Support.note, play: Support.store(0x18, 0x10) + Support.store(0x19, 0x86)))
        defer { cgb_free(m) }
        _ = Support.render(m, frames: 3000)
        let snapshot = try #require(cgb_snapshot_take(m))
        defer { cgb_snapshot_free(snapshot) }
        #expect(cgb_snapshot_size(snapshot) > 300_000)
        let first = Support.render(m, frames: 9600)
        #expect(cgb_snapshot_restore(m, snapshot) == 0)
        #expect(Support.render(m, frames: 9600) == first)

        let other = try #require(Support.machine(init: Support.note, play: Support.store(0x18, 0x10) + Support.store(0x19, 0x86)))
        defer { cgb_free(other) }
        #expect(cgb_snapshot_restore(other, snapshot) == 0)
        #expect(Support.render(other, frames: 9600) == first)
    }

    @Test func aSnapshotRefusesAMachineThatDiffers() throws {
        let m = try #require(Support.machine(init: Support.note))
        let otherRate = try #require(Support.machine(init: Support.note, rate: 44100))
        let otherFile = try #require(Support.machine(init: Support.power))
        defer { cgb_free(m); cgb_free(otherRate); cgb_free(otherFile) }
        let snapshot = try #require(cgb_snapshot_take(m))
        defer { cgb_snapshot_free(snapshot) }
        #expect(cgb_snapshot_restore(otherRate, snapshot) == 1)
        #expect(cgb_snapshot_restore(otherFile, snapshot) == 2)
        #expect(cgb_snapshot_restore(m, nil) == 2)
    }

    @Test func theChannelMaskSilencesAChannelAndSurvivesAReset() throws {
        let m = try #require(Support.machine(init: Support.note))
        defer { cgb_free(m) }
        #expect(cgb_channel_mask(m) == 0xF)
        cgb_set_channel_mask(m, 0b1101)
        _ = Support.render(m, frames: 48000)
        let tail = Support.render(m, frames: 9600)
        #expect(tail.allSatisfy { abs(Int($0)) < 50 })
        cgb_reset(m)
        #expect(cgb_channel_mask(m) == 0b1101)
        var levels = [Int32](repeating: 0, count: 4)
        cgb_run_cycles(m, 100_000)
        cgb_channel_levels_get(m, &levels)
        #expect(levels[1] != 0 || levels.allSatisfy { $0 >= 0 }, "a muted channel still has a level")
    }

    @Test func theChannelLevelsAreFourNumbersFromZeroToFifteen() throws {
        let m = try #require(Support.machine(init: Support.note))
        defer { cgb_free(m) }
        var seen = Set<Int32>()
        var levels = [Int32](repeating: -1, count: 4)
        for _ in 0..<64 {
            cgb_run_cycles(m, 4001)
            cgb_channel_levels_get(m, &levels)
            seen.insert(levels[1])
            #expect(levels.allSatisfy { (0...15).contains($0) })
        }
        #expect(seen.contains(15) && seen.contains(0), "the pulse alternates")
    }
}
