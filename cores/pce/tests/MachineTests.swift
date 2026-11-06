import Foundation
import Testing
import CPce

@Suite("Machines")
struct MachineTests {
    @Test func theSongNumberReachesTheProgramAndTheMaskMutesTheChannel() {
        let file = Support.hes(request: Support.songAsWord())
        let low = Support.machine(file, song: 3)
        let high = Support.machine(file, song: 28)
        defer { cpce_free(low); cpce_free(high) }
        _ = cpce_set_highpass_hz(low, 0)
        _ = cpce_set_highpass_hz(high, 0)
        #expect(Support.render(low, frames: 2000)[2 * 1500] < -0.05)
        #expect(Support.render(high, frames: 2000)[2 * 1500] > 0.05)
        #expect(cpce_channel_mask(high) == 0x3F)
        cpce_set_channel_mask(high, 0x3E)
        #expect(cpce_channel_mask(high) == 0x3E)
        _ = Support.render(high, frames: 64)
        #expect(Support.render(high, frames: 100).allSatisfy { abs($0) < 1e-6 })
    }

    @Test func theChannelLevelsShowAMutedChannel() {
        let m = Support.machine(Support.hes(request: Support.songAsWord()), song: 28)
        defer { cpce_free(m) }
        cpce_set_channel_mask(m, 0)
        _ = Support.render(m, frames: 100)
        var levels = [Int32](repeating: 0, count: 6)
        cpce_channel_levels_get(m, &levels)
        #expect(levels == [25, 0, 0, 0, 0, 0])
    }

    final class Seen {
        var writes: [(Int32, UInt32, UInt32, UInt64)] = []
        var insideSamples: [UInt64] = []
        var machine: OpaquePointer?
    }

    @Test func anObserverHearsTheStoresAndChangesNothing() {
        let file = Support.hes(request: Support.songAsWord())
        let plain = Support.machine(file, song: 9)
        defer { cpce_free(plain) }
        let expected = Support.render(plain, frames: 1000)

        let m = Support.machine(file, song: 9)
        defer { cpce_free(m) }
        let seen = Seen()
        seen.machine = m
        var observer = cpce_observer(
            context: Unmanaged.passUnretained(seen).toOpaque(),
            psg_write: { context, channel, reg, value, sample in
                let seen = Unmanaged<Seen>.fromOpaque(context!).takeUnretainedValue()
                seen.writes.append((channel, reg, value, sample))
                seen.insideSamples.append(cpce_samples(seen.machine))
                cpce_reset(seen.machine)
            })
        cpce_set_observer(m, &observer)
        let got = Support.render(m, frames: 1000)
        #expect(got == expected)
        #expect(seen.writes.count == 5)
        #expect(seen.writes[0] == (0, 0, 0, 0))
        #expect(seen.writes[1] == (0, 1, 0xFF, 0))
        #expect(seen.writes[4] == (0, 6, 9, 0))
        #expect(seen.insideSamples.allSatisfy { $0 == 0 })
        cpce_set_observer(m, nil)
        cpce_reset(m)
        _ = Support.render(m, frames: 100)
        #expect(seen.writes.count == 5)
    }

    @Test func aSnapshotRestoresTheState() {
        let file = Support.hes(request: Support.songAsWord())
        let m = Support.machine(file, song: 20)
        defer { cpce_free(m) }
        _ = Support.render(m, frames: 500)
        let snapshot = cpce_snapshot_take(m)
        defer { cpce_snapshot_free(snapshot) }
        let first = Support.render(m, frames: 800)
        #expect(cpce_snapshot_restore(m, snapshot) == 0)
        #expect(Support.render(m, frames: 800) == first)
        let other = Support.machine(file, song: 21)
        defer { cpce_free(other) }
        #expect(cpce_snapshot_restore(other, snapshot) == 2)
        #expect(cpce_snapshot_restore(m, nil) == 2)
    }

    @Test func resetStartsTheSongAgain() {
        let m = Support.machine(Support.hes(request: Support.songAsWord()), song: 2)
        defer { cpce_free(m) }
        let a = Support.render(m, frames: 700)
        _ = Support.render(m, frames: 3000)
        cpce_reset(m)
        #expect(cpce_samples(m) == 0)
        #expect(Support.render(m, frames: 700) == a)
    }
}
