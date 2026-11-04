import Foundation
import Testing
import CSnes

@Suite("Machines")
struct MachineTests {
    @Test func theRateIsChecked() {
        let image = Support.image(Support.tone())
        defer { csnes_image_free(image) }
        #expect(csnes_new(image, 0) == nil)
        #expect(csnes_new(nil, 48000) == nil)
        let m = csnes_new(image, 48000)
        #expect(m != nil)
        csnes_free(m)
    }

    @Test func aToneSoundsAndTheSameFileRendersTheSameSamples() {
        let a = Support.machine(Support.tone())
        let b = Support.machine(Support.tone())
        defer { csnes_free(a); csnes_free(b) }
        let x = Support.render(a, frames: 4000)
        let y = Support.render(b, frames: 4000)
        #expect(x == y)
        #expect(x.contains { $0 != 0 })
    }

    @Test func theMachineStartsWhereTheFileStands() {
        let m = Support.machine(Support.tone())
        defer { csnes_free(m) }
        #expect(csnes_register(m, 5) == UInt32(Support.start))
        #expect(csnes_register(m, 4) == 0xEF)
        #expect(csnes_dsp_register(m, 0x4C) == 0x01)
        #expect(csnes_dsp_register(m, 0xCC) == 0x01)
        #expect(csnes_peek(m, 0x2000) == 0xC3)
        #expect(csnes_cycles(m) == 0)
    }

    @Test func theClockAdvancesByTheCyclesRun() {
        let m = Support.machine(Support.tone())
        defer { csnes_free(m) }
        csnes_run_cycles(m, 32_000)
        #expect(csnes_cycles(m) >= 32_000)
        #expect(csnes_samples(m) == csnes_cycles(m) / 32)
        csnes_reset(m)
        #expect(csnes_cycles(m) == 0)
    }

    @Test func aSnapshotContinuesExactly() {
        let m = Support.machine(Support.tone())
        defer { csnes_free(m) }
        _ = Support.render(m, frames: 2000)
        let snap = csnes_snapshot_take(m)
        defer { csnes_snapshot_free(snap) }
        let after = Support.render(m, frames: 3000)
        #expect(csnes_snapshot_restore(m, snap) == 0)
        #expect(Support.render(m, frames: 3000) == after)
        #expect(csnes_snapshot_size(snap) > 65536)
    }

    @Test func aSnapshotRefusesAnotherRateOrFile() {
        let a = Support.machine(Support.tone())
        let other = Support.machine(Support.tone(), rate: 48000)
        let different = Support.machine(Support.spc(program: [0x00, 0x2F, 0xFD]))
        defer { csnes_free(a); csnes_free(other); csnes_free(different) }
        let snap = csnes_snapshot_take(a)
        defer { csnes_snapshot_free(snap) }
        #expect(csnes_snapshot_restore(other, snap) == 1)
        #expect(csnes_snapshot_restore(different, snap) == 2)
        #expect(csnes_snapshot_restore(a, nil) == 2)
    }

    @Test func theChannelMaskHasNineBitsAndCutsAVoiceFromTheMix() {
        let m = Support.machine(Support.tone())
        defer { csnes_free(m) }
        #expect(csnes_channel_mask(m) == 0x1FF)
        csnes_set_channel_mask(m, ~UInt64(1))
        #expect(csnes_channel_mask(m) == 0x1FE)
        #expect(Support.render(m, frames: 2000).allSatisfy { $0 == 0 })
        var levels = [Int32](repeating: 0, count: 8)
        csnes_voice_levels_get(m, &levels)
        #expect(levels[0] != 0)
        csnes_set_channel_mask(m, UInt64.max)
        #expect(csnes_channel_mask(m) == 0x1FF)
        #expect(Support.render(m, frames: 500).contains { $0 != 0 })
    }

    @Test func thePortsCarryValuesBothWays() {
        // MOV A,$F4 ; MOV $F5,A ; spin
        let m = Support.machine(Support.spc(program: [0xE4, 0xF4, 0xC4, 0xF5, 0x2F, 0xFE]))
        defer { csnes_free(m) }
        csnes_write_port(m, 0, 0x5A)
        csnes_run_cycles(m, 100)
        #expect(csnes_read_port(m, 1) == 0x5A)
        #expect(csnes_read_port(m, 9) == 0)
        csnes_write_port(m, 9, 1)
    }

    @Test func aHaltedProcessorShowsAsStalled() {
        let m = Support.machine(Support.spc(program: [0xEF]))
        defer { csnes_free(m) }
        #expect(csnes_end_state_get(m) == CSNES_RUNNING)
        csnes_run_cycles(m, 100)
        #expect(csnes_end_state_get(m) == CSNES_STALLED)
    }

    @Test func floatAndIntegerOutputAgree() {
        let a = Support.machine(Support.tone())
        let b = Support.machine(Support.tone())
        defer { csnes_free(a); csnes_free(b) }
        let ints = Support.render(a, frames: 1500)
        var floats = [Float](repeating: 0, count: 3000)
        floats.withUnsafeMutableBufferPointer { csnes_render_f32(b, $0.baseAddress, 1500) }
        for (i, f) in zip(ints, floats) {
            #expect(abs(Double(i) / 32768 - Double(f)) < 1e-4)
        }
    }
}
