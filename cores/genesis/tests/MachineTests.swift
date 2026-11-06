import Foundation
import Testing
import CGenesis

@Suite("Machines")
struct MachineTests {
    @Test func aSilentFileRendersSilence() {
        let m = Support.machine(Support.vgm(stream: Support.wait(44_100)))
        defer { cgenesis_free(m) }
        #expect(Support.render(m, frames: 4410).allSatisfy { $0 == 0 })
        #expect(cgenesis_end_state_get(m) == 0)
    }

    @Test func theFmToneSoundsAndTheMaskMutesIt() {
        let stream = Support.fmTone() + Support.wait(44_100)
        let m = Support.machine(Support.vgm(stream: stream, psg: false))
        defer { cgenesis_free(m) }
        _ = cgenesis_set_highpass_hz(m, 0)
        #expect(Support.rms(Support.render(m, frames: 22_050)) > 0.1)
        cgenesis_set_channel_mask(m, 0x3FF & ~1)
        #expect(cgenesis_channel_mask(m) == 0x3FE)
        _ = Support.render(m, frames: 64)
        #expect(Support.rms(Support.render(m, frames: 4410)) < 1e-6)
    }

    @Test func thePsgToneSoundsAndTheMaskMutesIt() {
        let stream = Support.psgTone() + Support.wait(44_100)
        let m = Support.machine(Support.vgm(stream: stream, fm: false))
        defer { cgenesis_free(m) }
        _ = cgenesis_set_highpass_hz(m, 0)
        #expect(Support.rms(Support.render(m, frames: 22_050)) > 0.05)
        cgenesis_set_channel_mask(m, 0x3FF & ~(1 << 6))
        _ = Support.render(m, frames: 64)
        #expect(Support.rms(Support.render(m, frames: 4410)) < 1e-6)
    }

    @Test func theChannelLevelsShowAMutedChannel() {
        var levels = [Int32](repeating: 0, count: 10)
        let fm = Support.machine(Support.vgm(stream: Support.fmTone() + Support.wait(44_100), psg: false))
        defer { cgenesis_free(fm) }
        cgenesis_set_channel_mask(fm, 0)
        _ = Support.render(fm, frames: 1500)
        cgenesis_channel_levels_get(fm, &levels)
        #expect(abs(levels[0]) > 10 && abs(levels[0]) <= 320)
        #expect(levels[1...].allSatisfy { $0 == 0 })
        let psg = Support.machine(Support.vgm(stream: Support.psgTone() + Support.wait(44_100), fm: false))
        defer { cgenesis_free(psg) }
        cgenesis_set_channel_mask(psg, 0)
        _ = Support.render(psg, frames: 500)
        cgenesis_channel_levels_get(psg, &levels)
        #expect(levels[..<6].allSatisfy { $0 == 0 })
        #expect(abs(levels[6]) == 8192)
    }

    @Test func aSongWithoutALoopEndsAndThenIsSilent() {
        let stream = Support.psgTone() + Support.wait(2205)
        let m = Support.machine(Support.vgm(stream: stream, fm: false))
        defer { cgenesis_free(m) }
        _ = cgenesis_set_highpass_hz(m, 0)
        _ = Support.render(m, frames: 8820)
        #expect(cgenesis_end_state_get(m) == 1)
        #expect(Support.render(m, frames: 4410).allSatisfy { $0 == 0 })
    }

    @Test func aLoopCountsItsPasses() {
        let stream = Support.psgTone() + Support.wait(441)
        let m = Support.machine(Support.vgm(stream: stream, fm: false, loopAt: 0))
        defer { cgenesis_free(m) }
        _ = Support.render(m, frames: 4410)
        #expect(cgenesis_loops(m) >= 9)
        #expect(cgenesis_end_state_get(m) == 0)
    }

    @Test func theSampleCountAdvancesWithTheRender() {
        let m = Support.machine(Support.vgm(stream: Support.wait(44_100)))
        defer { cgenesis_free(m) }
        _ = Support.render(m, frames: 22_050)
        #expect(abs(Int(cgenesis_samples(m)) - 22_050) <= 2)
    }

    @Test func theSameCallsGiveTheSameBytes() {
        let stream = Support.fmTone() + Support.psgTone() + Support.wait(22_050)
        let bytes = Support.vgm(stream: stream)
        let a = Support.machine(bytes), b = Support.machine(bytes)
        defer { cgenesis_free(a); cgenesis_free(b) }
        #expect(Support.render(a, frames: 10_000) == Support.render(b, frames: 10_000))
    }

    @Test func renderingInPiecesMatchesOneCall() {
        let stream = Support.fmTone() + Support.psgTone() + Support.wait(22_050)
        let bytes = Support.vgm(stream: stream)
        let a = Support.machine(bytes), b = Support.machine(bytes)
        defer { cgenesis_free(a); cgenesis_free(b) }
        let whole = Support.render(a, frames: 6000)
        let parts = Support.render(b, frames: 1) + Support.render(b, frames: 2999)
            + Support.render(b, frames: 3000)
        #expect(whole == parts)
    }

    @Test func resetStartsTheSongAgain() {
        let stream = Support.fmTone() + Support.wait(22_050)
        let m = Support.machine(Support.vgm(stream: stream, psg: false))
        defer { cgenesis_free(m) }
        let first = Support.render(m, frames: 5000)
        _ = Support.render(m, frames: 3000)
        cgenesis_reset(m)
        #expect(cgenesis_samples(m) == 0)
        #expect(Support.render(m, frames: 5000) == first)
    }

    @Test func aSnapshotRestoresTheState() {
        let stream = Support.fmTone() + Support.psgTone() + Support.wait(44_100)
        let m = Support.machine(Support.vgm(stream: stream))
        defer { cgenesis_free(m) }
        _ = Support.render(m, frames: 3000)
        let snapshot = cgenesis_snapshot_take(m)!
        defer { cgenesis_snapshot_free(snapshot) }
        let next = Support.render(m, frames: 4000)
        #expect(cgenesis_snapshot_restore(m, snapshot) == 0)
        #expect(Support.render(m, frames: 4000) == next)
    }

    @Test func aSnapshotOfAnotherFileIsRefused() {
        let a = Support.machine(Support.vgm(stream: Support.wait(100)))
        let b = Support.machine(Support.vgm(stream: Support.wait(200)))
        defer { cgenesis_free(a); cgenesis_free(b) }
        let snapshot = cgenesis_snapshot_take(a)!
        defer { cgenesis_snapshot_free(snapshot) }
        #expect(cgenesis_snapshot_restore(b, snapshot) == 2)
        #expect(cgenesis_snapshot_restore(b, nil) == 2)
    }

    @Test func aSnapshotRestoresIntoAnotherMachineOfTheSameImage() {
        let image = Support.image(Support.vgm(stream: Support.fmTone() + Support.wait(44_100)))
        defer { cgenesis_image_free(image) }
        let a = cgenesis_new(image, 44_100)!
        let b = cgenesis_new(image, 44_100)!
        defer { cgenesis_free(a); cgenesis_free(b) }
        _ = Support.render(a, frames: 500)
        let snapshot = cgenesis_snapshot_take(a)!
        defer { cgenesis_snapshot_free(snapshot) }
        #expect(cgenesis_snapshot_restore(b, snapshot) == 0)
        #expect(Support.render(b, frames: 800) == Support.render(a, frames: 800))
    }

    @Test func aRateOutOfRangeMakesNoMachine() {
        let image = Support.image(Support.vgm(stream: Support.wait(100)))
        defer { cgenesis_image_free(image) }
        #expect(cgenesis_new(image, 7999) == nil)
        #expect(cgenesis_new(image, 192_001) == nil)
        #expect(cgenesis_new(nil, 44_100) == nil)
    }

    @Test func theHighPassIsOnByDefaultAndTakesOnlyASaneCutoff() {
        let m = Support.machine(Support.vgm(stream: Support.wait(100)))
        defer { cgenesis_free(m) }
        #expect(cgenesis_highpass_hz(m) == 20)
        #expect(cgenesis_set_highpass_hz(m, 55) == 1)
        #expect(cgenesis_set_highpass_hz(m, -1) == 0)
        #expect(cgenesis_set_highpass_hz(m, 1001) == 0)
        #expect(cgenesis_highpass_hz(m) == 55)
        cgenesis_reset(m)
        #expect(cgenesis_highpass_hz(m) == 55)
    }

    @Test func aYm2413FilePlaysAndHasItsOwnMaskBits() {
        // A user tone: one sine carrier at about 440 Hz on channel 1.
        var stream: [UInt8] = []
        let tone: [UInt8] = [0x21, 0x21, 0x3F, 0x00, 0xF0, 0xF0, 0x0F, 0x0F]
        for (i, v) in tone.enumerated() { stream += [0x51, UInt8(i), v] }
        stream += [0x51, 0x30, 0x00, 0x51, 0x10, 0x22, 0x51, 0x20, 0x19] + Support.wait(44_100)
        var bytes = Support.vgm(stream: stream, psg: false, fm: false)
        bytes.replaceSubrange(0x10..<0x14, with: [0x99, 0x9E, 0x36, 0x00])
        let image = Support.image(bytes)
        defer { cgenesis_image_free(image) }
        #expect(cgenesis_image_chips(image) == 4)
        let m = cgenesis_new(image, 44_100)!
        defer { cgenesis_free(m) }
        _ = cgenesis_set_highpass_hz(m, 0)
        #expect(cgenesis_channel_mask(m) == 0x3FF | 0x1FF << 20)
        #expect(Support.rms(Support.render(m, frames: 22_050)) > 0.02)
        cgenesis_set_channel_mask(m, 0x3FF)
        _ = Support.render(m, frames: 64)
        #expect(Support.rms(Support.render(m, frames: 4410)) < 1e-6)
    }

    final class Seen {
        var writes: [(Int32, UInt32, UInt32, UInt64)] = []
        var insideSamples: [UInt64] = []
        var machine: OpaquePointer?
    }

    @Test func anObserverHearsTheRegisterWritesAndChangesNothing() {
        var stream: [UInt8] = [0x50, 0x9F, 0x52, 0x28, 0xF0] + Support.wait(441)
        stream += [0x53, 0xB4, 0xC0] + Support.wait(441)
        let bytes = Support.vgm(stream: stream)
        let plain = Support.machine(bytes)
        defer { cgenesis_free(plain) }
        let expected = Support.render(plain, frames: 1500)

        let m = Support.machine(bytes)
        defer { cgenesis_free(m) }
        let seen = Seen()
        seen.machine = m
        var observer = cgenesis_observer(
            context: Unmanaged.passUnretained(seen).toOpaque(),
            register_write: { context, chip, address, value, sample in
                let seen = Unmanaged<Seen>.fromOpaque(context!).takeUnretainedValue()
                seen.writes.append((chip, address, value, sample))
                seen.insideSamples.append(cgenesis_samples(seen.machine))
                cgenesis_reset(seen.machine)
            })
        cgenesis_set_observer(m, &observer)
        let got = Support.render(m, frames: 1500)
        #expect(got == expected)
        #expect(seen.writes.count == 3)
        #expect(seen.writes[0] == (Int32(CGENESIS_CHIP_SN76489.rawValue), 0, 0x9F, 0))
        #expect(seen.writes[1] == (Int32(CGENESIS_CHIP_YM2612.rawValue), 0x028, 0xF0, 0))
        #expect(seen.writes[2] == (Int32(CGENESIS_CHIP_YM2612.rawValue), 0x1B4, 0xC0, 441))
        #expect(seen.insideSamples == [0, 0, 441])
        cgenesis_set_observer(m, nil)
        cgenesis_reset(m)
        _ = Support.render(m, frames: 100)
        #expect(seen.writes.count == 3)
    }

    @Test func theVoicesReportTheirStateAndShowInTheirScopes() {
        let stream = Support.fmTone() + Support.psgTone() + Support.wait(44_100)
        let m = Support.machine(Support.vgm(stream: stream))
        defer { cgenesis_free(m) }
        _ = cgenesis_set_highpass_hz(m, 0)
        _ = Support.render(m, frames: 8000)
        #expect(cgenesis_voice_count(m) == 10)
        var voices = [cgenesis_voice](repeating: cgenesis_voice(), count: 10)
        #expect(cgenesis_voices_get(m, &voices, 10) == 10)
        #expect(voices[0].kind == 3 && voices[0].key_on == 1 && voices[0].active == 1)
        #expect(abs(voices[0].frequency_hz - 238.5) < 2)
        #expect(voices[0].envelope > 0.9 && abs(voices[0].pan) < 0.001)
        #expect(voices[0].mask_bit == 0 && voices[6].mask_bit == 6)
        #expect(voices[6].kind == 0 && voices[6].key_on == 1)
        #expect(voices[9].kind == 1)
        #expect(voices[1].key_on == 0 && voices[1].active == 0)
        #expect(String(cString: cgenesis_voice_name(m, 0)) == "FM 1")
        #expect(String(cString: cgenesis_voice_name(m, 6)) == "PSG Tone 1")
        #expect(cgenesis_voice_name(m, 10) == nil)
        #expect(cgenesis_voices_get(m, &voices, 4) == 4)
        #expect(cgenesis_voices_get(m, &voices, 0) == 0)
        cgenesis_set_channel_mask(m, 0)
        _ = Support.render(m, frames: 500)
        var scope = [Float](repeating: 0, count: 2048)
        #expect(cgenesis_scope_get(m, 0, &scope, 2048) == 2048)
        #expect(scope.contains { $0 > 0.05 } && scope.contains { $0 < -0.05 })
        #expect(cgenesis_scope_get(m, 10, &scope, 64) == 0)
        #expect(cgenesis_scope_get(m, -1, &scope, 64) == 0)
        #expect(cgenesis_scope_get(m, 0, nil, 64) == 0)
        #expect(cgenesis_scope_get(m, 0, &scope, 20_000) == 0)
        #expect(cgenesis_voice_count(nil) == 0)
    }
}
