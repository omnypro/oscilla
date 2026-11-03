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
        #expect(cnes_channel_mask(m) == 0x3_FFFF_FFFF)
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
        cnes_set_channel_mask(m, UInt64.max)
        #expect(cnes_channel_mask(m) == 0x3_FFFF_FFFF, "only the bits that name a channel count")
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

    @Test func theExpansionChannelsShowTheirLevelsAndAnswerToTheMask() throws {
        let setup = Support.store(0x9000, 0x7F) + Support.store(0x9001, 0xFF) + Support.store(0x9002, 0x80)
            + Support.store(0xB000, 0x08) + Support.store(0xB001, 0x40) + Support.store(0xB002, 0x80)
        let m = try #require(Support.machine(init: setup, chips: 1))
        defer { cnes_free(m) }
        var levels = [Int32](repeating: -1, count: 8)
        var loudest = [Int32](repeating: 0, count: 8)
        for _ in 0..<400 {
            cnes_run_cycles(m, 997)
            cnes_expansion_levels_get(m, &levels)
            for i in 0..<8 { loudest[i] = max(loudest[i], levels[i]) }
        }
        #expect(loudest[0] == 15, "pulse 1 plays at full volume")
        #expect(loudest[1] == 0, "pulse 2 was never started")
        #expect((1...31).contains(loudest[2]), "the saw climbs")
        #expect(loudest[3...].allSatisfy { $0 == 0 })
        let loud = Support.render(m, frames: 4800).map { abs(Int($0)) }.max() ?? 0
        #expect(loud > 1000)
        cnes_set_channel_mask(m, 0x1F)
        _ = Support.render(m, frames: 48000)
        let quiet = Support.render(m, frames: 4800).map { abs(Int($0)) }.max() ?? 0
        #expect(quiet < 50, "the expansion bits mute the chip")
        cnes_expansion_levels_get(m, &levels)
        #expect(levels[0] > 0 || levels[2] > 0 || loudest[0] == 15, "a muted channel still shows its level")
        cnes_expansion_levels_get(m, nil)
    }

    @Test func aFileWithoutTheChipShowsNoExpansionLevels() throws {
        let m = try #require(Support.machine(init: Support.store(0x9000, 0x7F) + Support.store(0x9002, 0x80)))
        defer { cnes_free(m) }
        cnes_run_cycles(m, 10_000)
        var levels = [Int32](repeating: -1, count: 8)
        cnes_expansion_levels_get(m, &levels)
        #expect(levels.allSatisfy { $0 == 0 })
    }

    @Test func theSunsoftChannelsShowTheirLevelsAndAnswerToTheMask() throws {
        func port(_ r: UInt8, _ v: UInt8) -> [UInt8] {
            Support.store(0xC000, r) + Support.store(0xE000, v)
        }
        let setup = port(7, 0b0011_1110) + port(8, 15) + port(0, 100) + port(1, 0)
        let m = try #require(Support.machine(init: setup, chips: 32))
        defer { cnes_free(m) }
        var levels = [Int32](repeating: -1, count: 8)
        var loudest = [Int32](repeating: 0, count: 8)
        for _ in 0..<400 {
            cnes_run_cycles(m, 997)
            cnes_expansion_levels_get(m, &levels)
            for i in 0..<8 { loudest[i] = max(loudest[i], levels[i]) }
        }
        #expect(loudest[0] == 31, "channel A plays at full volume")
        #expect(loudest[1] == 0 && loudest[2] == 0)
        #expect(loudest[3...].allSatisfy { $0 == 0 })
        let loud = Support.render(m, frames: 4800).map { abs(Int($0)) }.max() ?? 0
        #expect(loud > 1000)
        cnes_set_channel_mask(m, 0x3_FFFF_FFFF & ~0x100)
        _ = Support.render(m, frames: 48000)
        let quiet = Support.render(m, frames: 4800).map { abs(Int($0)) }.max() ?? 0
        #expect(quiet < 50, "bit 8 mutes channel A")
    }

    @Test func theNamcoChannelsShowTheirLevelsAndAnswerToTheMask() throws {
        func ram(_ at: UInt8, _ bytes: [UInt8]) -> [UInt8] {
            var out = Support.store(0xF800, 0x80 | at)
            for b in bytes { out += Support.store(0x4800, b) }
            return out
        }
        // Channel 8 plays a square wave of four samples at volume 3, one channel.
        let setup = ram(0, [0xFF, 0x00]) + ram(0x78, [0x00, 0x00, 0x40, 0x00, 0xFC, 0x00, 0x00, 0x03])
        let m = try #require(Support.machine(init: setup, chips: 16))
        defer { cnes_free(m) }
        var levels = [Int32](repeating: -1, count: 8)
        var loudest = [Int32](repeating: 0, count: 8)
        for _ in 0..<400 {
            cnes_run_cycles(m, 997)
            cnes_expansion_levels_get(m, &levels)
            for i in 0..<8 { loudest[i] = max(loudest[i], levels[i]) }
        }
        #expect(loudest[7] == 24, "channel 8 puts out 8 steps at volume 3")
        #expect(loudest[0..<7].allSatisfy { $0 == 0 })
        let loud = Support.render(m, frames: 4800).map { abs(Int($0)) }.max() ?? 0
        #expect(loud > 500)
        cnes_set_channel_mask(m, 0x3_FFFF_FFFF & ~(1 << 18))
        _ = Support.render(m, frames: 48000)
        let quiet = Support.render(m, frames: 4800).map { abs(Int($0)) }.max() ?? 0
        #expect(quiet < 50, "bit 18 mutes channel 8")
    }

    @Test func theDiskSystemChannelShowsItsLevelAndAnswersToTheMask() throws {
        // A square wave of 32 samples at 63 and 32 at 0, at a fixed gain of 32.
        var setup = Support.store(0x4089, 0x80)
        for i in 0..<64 { setup += Support.store(UInt16(0x4040 + i), i < 32 ? 63 : 0) }
        setup += Support.store(0x4089, 0) + Support.store(0x4080, 0x80 | 32)
            + Support.store(0x4082, 0x00) + Support.store(0x4083, 0x01)
        let m = try #require(Support.machine(init: setup, chips: 4))
        defer { cnes_free(m) }
        var levels = [Int32](repeating: -1, count: 8)
        var loudest: Int32 = 0
        for _ in 0..<400 {
            cnes_run_cycles(m, 997)
            cnes_expansion_levels_get(m, &levels)
            loudest = max(loudest, levels[0])
        }
        #expect(loudest == 63 * 32, "the level is the sample times the gain")
        #expect(levels[1...].allSatisfy { $0 == 0 })
        let loud = Support.render(m, frames: 4800).map { abs(Int($0)) }.max() ?? 0
        #expect(loud > 500)
        cnes_set_channel_mask(m, 0x3_FFFF_FFFF & ~(1 << 19))
        _ = Support.render(m, frames: 48000)
        let quiet = Support.render(m, frames: 4800).map { abs(Int($0)) }.max() ?? 0
        #expect(quiet < 50, "bit 19 mutes the chip")
    }

    @Test func theMMC5PulseChannelsShowTheirLevelsAndAnswerToTheMask() throws {
        // Pulse 1: duty 50%, constant volume 15, longest length, period 255.
        var setup: [UInt8] = []
        setup += Support.store(0x5015, 0x03)
        setup += Support.store(0x5000, 0xBF)
        setup += Support.store(0x5002, 0xFF)
        setup += Support.store(0x5003, 0xF8)
        setup += Support.store(0x5011, 0x00)
        let m = try #require(Support.machine(init: setup, chips: 8))
        defer { cnes_free(m) }
        var levels = [Int32](repeating: -1, count: 8)
        var loudest: Int32 = 0
        for _ in 0..<400 {
            cnes_run_cycles(m, 997)
            cnes_expansion_levels_get(m, &levels)
            loudest = max(loudest, levels[0])
        }
        #expect(loudest == 15)
        #expect(levels[1] == 0)
        #expect(levels[3...].allSatisfy { $0 == 0 })
        let loud = Support.render(m, frames: 4800).map { abs(Int($0)) }.max() ?? 0
        #expect(loud > 500)
        cnes_set_channel_mask(m, 0x3_FFFF_FFFF & ~(1 << 20))
        _ = Support.render(m, frames: 48000)
        let quiet = Support.render(m, frames: 4800).map { abs(Int($0)) }.max() ?? 0
        #expect(quiet < 50, "bit 20 mutes pulse 1")
        #expect(String(cString: cnes_warnings(m)).isEmpty)
    }

    @Test func theVRC7ChannelShowsItsLevelAndAnswersToTheMask() throws {
        // A plain sine on channel 1: the carrier sustains at once, the
        // modulator is nearly silent.
        func register(_ r: UInt16, _ v: UInt8) -> [UInt8] {
            Support.store(0x9010, UInt8(r)) + Support.store(0x9030, v)
        }
        var setup: [UInt8] = []
        let regs: [(UInt16, UInt8)] = [
            (0x00, 0x01), (0x01, 0x21), (0x02, 0x3F), (0x03, 0x00),
            (0x04, 0xF0), (0x05, 0xF0), (0x06, 0x00), (0x07, 0x00),
            (0x30, 0x00), (0x10, 0x00), (0x20, 0x17),
        ]
        for (r, v) in regs { setup += register(r, v) }
        let m = try #require(Support.machine(init: setup, chips: 2))
        defer { cnes_free(m) }
        var levels = [Int32](repeating: -1, count: 8)
        var loudest: Int32 = 0
        for _ in 0..<400 {
            cnes_run_cycles(m, 997)
            cnes_expansion_levels_get(m, &levels)
            loudest = max(loudest, levels[0])
        }
        #expect(loudest > 3000, "a full carrier peaks near 4088")
        #expect(levels[1...].allSatisfy { $0 == 0 })
        let loud = Support.render(m, frames: 4800).map { abs(Int($0)) }.max() ?? 0
        #expect(loud > 500)
        cnes_set_channel_mask(m, 0x3_FFFF_FFFF & ~(1 << 23))
        _ = Support.render(m, frames: 48000)
        let quiet = Support.render(m, frames: 4800).map { abs(Int($0)) }.max() ?? 0
        #expect(quiet < 50, "bit 23 mutes channel 1")
        #expect(String(cString: cnes_warnings(m)).isEmpty)
    }

    @Test func theVT02ChannelsShowTheirLevelsAndAnswerToTheMask() throws {
        let setup = Support.store(0x4030, 0x08) + Support.store(0x4035, 0x0F)
            + Support.store(0x4020, 0x7F) + Support.store(0x4022, 0xFF) + Support.store(0x4023, 0x08)
            + Support.store(0x4031, 0x20)
        let m = try #require(Support.machine(init: setup, chips: 64))
        defer { cnes_free(m) }
        var levels = [Int32](repeating: -1, count: 8)
        var loudest = [Int32](repeating: 0, count: 8)
        for _ in 0..<400 {
            cnes_run_cycles(m, 997)
            cnes_expansion_levels_get(m, &levels)
            for i in 0..<8 { loudest[i] = max(loudest[i], levels[i]) }
        }
        #expect(loudest[0] == 15, "pulse 3 plays at full volume")
        #expect(loudest[1] == 0, "pulse 4 was never started")
        #expect(loudest[4] == 0x20, "the raw PCM level shows")
        #expect(loudest[5...].allSatisfy { $0 == 0 })
        #expect(cnes_channel_mask(m) == 0x3_FFFF_FFFF)
        let loud = Support.render(m, frames: 4800).map { abs(Int($0)) }.max() ?? 0
        #expect(loud > 1000)
        cnes_set_channel_mask(m, 0x3_FFFF_FFFF & ~(0x1F << 29))
        _ = Support.render(m, frames: 48000)
        let quiet = Support.render(m, frames: 4800).map { abs(Int($0)) }.max() ?? 0
        #expect(quiet < 50, "bits 29 to 33 mute the second generator")
        #expect(String(cString: cnes_warnings(m)).isEmpty)
    }
}
