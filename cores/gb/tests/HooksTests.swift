import Testing
import CGb

/// The observer, snapshots, the channel mask and the bass cutoff, through the C interface.
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
                log.inside.append(UInt32(truncatingIfNeeded: cgb_set_highpass_hz(log.machine, 200)))
                log.inside.append(UInt32(cgb_highpass_hz(log.machine)))
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
        #expect((300_000...400_000).contains(cgb_snapshot_size(snapshot)))
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
        var loudest: Int32 = 0
        for _ in 0..<64 {
            cgb_run_cycles(m, 4001)
            cgb_channel_levels_get(m, &levels)
            loudest = max(loudest, levels[1])
        }
        #expect(loudest == 15, "a muted channel still shows its level")
    }

    @Test func theBassCutoffStartsAtTheConsoleAndAcceptsOnlyItsRange() throws {
        let m = try #require(Support.machine(init: Support.note))
        defer { cgb_free(m) }
        #expect(cgb_highpass_hz(m) == CGB_HIGHPASS_CONSOLE_HZ)
        #expect(cgb_set_highpass_hz(m, 300) == 1)
        #expect(cgb_highpass_hz(m) == 300)
        for bad in [0, CGB_HIGHPASS_MIN_HZ - 0.5, CGB_HIGHPASS_MAX_HZ + 0.5, -5, .nan, .infinity] {
            #expect(cgb_set_highpass_hz(m, bad) == 0, "\(bad)")
            #expect(cgb_highpass_hz(m) == 300)
        }
        #expect(cgb_set_highpass_hz(m, CGB_HIGHPASS_MIN_HZ) == 1)
        #expect(cgb_set_highpass_hz(m, CGB_HIGHPASS_MAX_HZ) == 1)
        cgb_use_console_highpass(m)
        #expect(cgb_highpass_hz(m) == CGB_HIGHPASS_CONSOLE_HZ)
    }

    @Test func theBassCutoffChangesTheSoundDuringPlayAndSurvivesAReset() throws {
        let plain = try #require(Support.machine(init: Support.note))
        let thin = try #require(Support.machine(init: Support.note))
        defer { cgb_free(plain); cgb_free(thin) }
        _ = Support.render(plain, frames: 4800)
        _ = Support.render(thin, frames: 4800)
        #expect(cgb_set_highpass_hz(thin, 1000) == 1)
        let a = Support.render(plain, frames: 9600)
        let b = Support.render(thin, frames: 9600)
        #expect(a != b)
        cgb_reset(thin)
        #expect(cgb_highpass_hz(thin) == 1000)
        cgb_use_console_highpass(thin)
        cgb_reset(thin)
        cgb_reset(plain)
        #expect(Support.render(thin, frames: 9600) == Support.render(plain, frames: 9600))
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
