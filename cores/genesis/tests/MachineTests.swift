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
        let b = Support.machine(Support.vgm(stream: Support.wait(100)))
        defer { cgenesis_free(a); cgenesis_free(b) }
        let snapshot = cgenesis_snapshot_take(a)!
        defer { cgenesis_snapshot_free(snapshot) }
        #expect(cgenesis_snapshot_restore(b, snapshot) == 2)
        #expect(cgenesis_snapshot_restore(b, nil) == 2)
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
}
