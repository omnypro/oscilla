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

    @Test func aSnapshotRestoresIntoAnotherMachineOfTheSameImage() {
        let image = Support.image(Support.hes(request: Support.songAsWord()))
        defer { cpce_image_free(image) }
        let a = cpce_new(image, 44100, 5)!
        let b = cpce_new(image, 44100, 5)!
        defer { cpce_free(a); cpce_free(b) }
        _ = Support.render(a, frames: 300)
        let snapshot = cpce_snapshot_take(a)!
        defer { cpce_snapshot_free(snapshot) }
        #expect(cpce_snapshot_restore(b, snapshot) == 0)
        #expect(Support.render(b, frames: 500) == Support.render(a, frames: 500))
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

    @Test func theChannelsReportTheirStateAndShowInTheirScopes() {
        // Channel 1 as a direct D/A channel at full level and the word 31, and
        // channel 2 as a waveform channel on with the default waveform.
        let psg: UInt16 = 0x0800
        var code: [UInt8] = []
        code += Support.sta(psg, 0) + Support.sta(psg + 1, 0xFF) + Support.sta(psg + 5, 0xFF)
        code += Support.sta(psg + 4, 0xDF) + Support.sta(psg + 6, 31)
        code += Support.sta(psg, 1) + Support.sta(psg + 5, 0xFF) + Support.sta(psg + 4, 0x40)
        code += Support.sta(psg + 4, 0x00)
        for i in 0..<32 { code += Support.sta(psg + 6, i < 16 ? 31 : 0) }
        code += Support.sta(psg + 2, 254) + Support.sta(psg + 3, 0) + Support.sta(psg + 4, 0x9F)
        code.append(0x60)
        let m = Support.machine(Support.hes(request: code), song: 0, rate: 44100)
        defer { cpce_free(m) }
        _ = cpce_set_highpass_hz(m, 0)
        _ = Support.render(m, frames: 6000)
        #expect(cpce_voice_count(m) == 6)
        var voices = [cpce_voice](repeating: cpce_voice(), count: 6)
        #expect(cpce_voices_get(m, &voices, 6) == 6)
        #expect(voices[0].kind == 2 && voices[0].active == 1 && voices[0].instrument == -1)
        #expect(voices[1].kind == 0 && voices[1].key_on == 1)
        #expect(abs(voices[1].frequency_hz - 440.45) < 0.1)
        #expect(voices[1].envelope == -1 && abs(voices[1].pan) < 0.001)
        #expect(voices[1].instrument >= 0)
        #expect(voices[2].active == 0 && voices[2].key_on == 0)
        #expect(voices[4].mask_bit == 4)
        #expect(String(cString: cpce_voice_name(m, 4)) == "Wave/Noise 5")
        #expect(cpce_voice_name(m, 6) == nil)
        cpce_set_channel_mask(m, 0x3D)
        _ = Support.render(m, frames: 500)
        var scope = [Float](repeating: 0, count: 2048)
        #expect(cpce_scope_get(m, 1, &scope, 2048) == 2048)
        #expect(scope.contains { $0 > 0.1 } && scope.contains { $0 < -0.1 })
        var idle = [Float](repeating: 1, count: 64)
        #expect(cpce_scope_get(m, 3, &idle, 64) == 64)
        #expect(idle.allSatisfy { $0 == 0 })
        #expect(cpce_scope_get(m, 6, &scope, 64) == 0)
        #expect(cpce_scope_get(m, 1, nil, 64) == 0)
        #expect(cpce_scope_get(m, 1, &scope, 20_000) == 0)
        #expect(cpce_voice_count(nil) == 0)
    }
}
