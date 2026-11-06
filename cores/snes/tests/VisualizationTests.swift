import Foundation
import Testing
import CSnes

@Suite("Visualization")
struct VisualizationTests {
    @Test func aPlayingVoiceReportsItsStateAndTheOthersAreIdle() {
        let m = Support.machine(Support.tone())
        defer { csnes_free(m) }
        _ = Support.render(m, frames: 2000)
        #expect(csnes_voice_count(m) == 8)
        var voices = [csnes_voice](repeating: csnes_voice(), count: 8)
        #expect(csnes_voices_get(m, &voices, 8) == 8)
        let v = voices[0]
        #expect(v.active == 1 && v.key_on == 1 && v.kind == 2)
        #expect(v.instrument == 0)
        #expect(abs(v.frequency_hz - 32000) < 0.5)
        #expect(v.envelope > 0.9 && v.envelope <= 1)
        #expect(abs(v.pan) < 0.001)
        #expect(abs(v.left - 0.5 * v.envelope) < 0.001)
        #expect(v.volume > 0.4)
        #expect(voices[3].active == 0 && voices[3].envelope == 0)
        #expect(String(cString: csnes_voice_name(m, 0)) == "Voice 1")
        #expect(csnes_voice_name(m, 8) == nil)
        #expect(csnes_voices_get(m, &voices, 3) == 3)
        #expect(csnes_voices_get(m, &voices, 0) == 0)
    }

    @Test func aScopeHoldsTheVoiceAndAMutedVoiceStillShows() {
        let m = Support.machine(Support.tone(), rate: 44100)
        defer { csnes_free(m) }
        _ = Support.render(m, frames: 4000)
        csnes_set_channel_mask(m, 0x1FE)
        _ = Support.render(m, frames: 500)
        var scope = [Float](repeating: 0, count: 1024)
        #expect(csnes_scope_get(m, 0, &scope, 1024) == 1024)
        #expect(scope[1023] > 0.01)
        var idle = [Float](repeating: 1, count: 64)
        #expect(csnes_scope_get(m, 5, &idle, 64) == 64)
        #expect(idle.allSatisfy { $0 == 0 })
        #expect(csnes_scope_get(m, 8, &scope, 64) == 0)
        #expect(csnes_scope_get(m, 0, nil, 64) == 0)
        #expect(csnes_scope_get(m, 0, &scope, 0) == 0)
        #expect(csnes_scope_get(m, 0, &scope, 20_000) == 0)
    }

    @Test func theAccessMapIsOffUntilTurnedOnAndShowsTheProgram() {
        // MOV A,$30 ; MOV $31,A ; BRA back
        let m = Support.machine(Support.spc(program: [0xE4, 0x30, 0xC4, 0x31, 0x2F, 0xFA]))
        defer { csnes_free(m) }
        var map = [UInt8](repeating: 9, count: 65536)
        #expect(csnes_access_tracking(m) == 0)
        #expect(csnes_access_map_get(m, &map) == 0)
        #expect(map.allSatisfy { $0 == 0 })
        csnes_set_access_tracking(m, 1)
        #expect(csnes_access_tracking(m) == 1)
        csnes_run_cycles(m, 2000)
        #expect(csnes_access_map_take(m, &map) == 1)
        #expect(map[Support.start] & UInt8(CSNES_ACCESS_EXEC) != 0)
        #expect(map[0x30] & UInt8(CSNES_ACCESS_READ) != 0)
        #expect(map[0x31] & UInt8(CSNES_ACCESS_WRITE) != 0)
        #expect(csnes_access_map_get(m, &map) == 1)
        #expect(map.allSatisfy { $0 == 0 })
        csnes_run_cycles(m, 200)
        csnes_access_map_clear(m)
        #expect(csnes_access_map_get(m, &map) == 1)
        #expect(map.allSatisfy { $0 == 0 })
        csnes_set_access_tracking(m, 0)
        #expect(csnes_access_tracking(m) == 0)
    }

    @Test func nothingHereWorksFromInsideACallbackAndNullIsSafe() {
        #expect(csnes_voice_count(nil) == 0)
        #expect(csnes_voice_name(nil, 0) == nil)
        var scope = [Float](repeating: 0, count: 8)
        #expect(csnes_scope_get(nil, 0, &scope, 8) == 0)
        #expect(csnes_access_tracking(nil) == 0)
    }
}
