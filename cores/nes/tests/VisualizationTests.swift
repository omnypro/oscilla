import Foundation
import Testing
import CNes

/// The voices and the scopes, through the C interface.
@Suite("Visualization")
struct VisualizationTests {
    @Test func aPulseReportsItsStateAndTheOthersAreIdle() throws {
        let m = try #require(Support.machine(init: Support.note))
        defer { cnes_free(m) }
        _ = Support.render(m, frames: 6000)
        #expect(cnes_voice_count(m) == 5)
        var voices = [cnes_voice](repeating: cnes_voice(), count: 5)
        #expect(cnes_voices_get(m, &voices, 5) == 5)
        let v = voices[0]
        #expect(v.kind == 0 && v.active == 1 && v.key_on == 1)
        #expect(v.instrument == 2 && v.mask_bit == 0)
        #expect(abs(v.frequency_hz - 436.95) < 0.05)
        #expect(v.volume == 1 && v.left == 1 && v.right == 1 && v.pan == 0)
        #expect(v.envelope == -1 && v.envelope_phase == 0 && v.pitch_modulated == 0)
        #expect(voices[1].active == 0 && voices[1].key_on == 0 && voices[1].volume == 0)
        #expect(voices[3].kind == 1 && voices[4].kind == 2 && voices[4].mask_bit == 4)
        #expect(String(cString: cnes_voice_name(m, 0)) == "Pulse 1")
        #expect(String(cString: cnes_voice_name(m, 4)) == "DMC")
        #expect(cnes_voice_name(m, 5) == nil)
        #expect(cnes_voice_name(m, -1) == nil)
        #expect(cnes_voices_get(m, &voices, 3) == 3)
        #expect(cnes_voices_get(m, &voices, 0) == 0)
    }

    @Test func theExpansionVoicesFollowTheChipsInTheOrderOfTheMask() throws {
        // The VRC6's pulse 1 at 437 Hz in a file with the VRC6 and the N163.
        let setup = Support.store(0x9000, 0x7F) + Support.store(0x9001, 0xFF) + Support.store(0x9002, 0x80)
        let m = try #require(Support.machine(init: setup, chips: 1 | 16))
        defer { cnes_free(m) }
        _ = Support.render(m, frames: 6000)
        #expect(cnes_voice_count(m) == 16)
        var voices = [cnes_voice](repeating: cnes_voice(), count: 16)
        #expect(cnes_voices_get(m, &voices, 16) == 16)
        #expect(voices[5].kind == 0 && voices[5].active == 1 && voices[5].instrument == 7)
        #expect(abs(voices[5].frequency_hz - 436.95) < 0.05)
        #expect(voices[5].mask_bit == 5 && voices[8].mask_bit == 11 && voices[15].mask_bit == 18)
        #expect(String(cString: cnes_voice_name(m, 7)) == "VRC6 Saw")
        #expect(String(cString: cnes_voice_name(m, 8)) == "N163 1")
        #expect(cnes_voice_name(m, 16) == nil)
        var scope = [Float](repeating: 0, count: 2048)
        #expect(cnes_scope_get(m, 5, &scope, 2048) == 2048)
        #expect(scope.contains { $0 > 0.5 } && scope.contains { $0 < -0.5 })
    }

    @Test func aScopeFollowsTheVoiceAndAMutedVoiceStillShows() throws {
        let m = try #require(Support.machine(init: Support.note))
        defer { cnes_free(m) }
        _ = Support.render(m, frames: 6000)
        cnes_set_channel_mask(m, 0x3_FFFF_FFFE)
        _ = Support.render(m, frames: 500)
        var scope = [Float](repeating: 0, count: 4096)
        #expect(cnes_scope_get(m, 0, &scope, 4096) == 4096)
        #expect(scope.contains { $0 > 0.5 } && scope.contains { $0 < -0.5 })
        // 437 Hz over 4,096 frames of 48,000 Hz: about 75 crossings.
        let crossings = zip(scope, scope.dropFirst()).filter { ($0 < 0) != ($1 < 0) }.count
        #expect((72...77).contains(crossings), "\(crossings)")
        var idle = [Float](repeating: 1, count: 64)
        #expect(cnes_scope_get(m, 1, &idle, 64) == 64)
        #expect(idle.allSatisfy { $0 == 0 })
        #expect(cnes_scope_get(m, 5, &scope, 64) == 0)
        #expect(cnes_scope_get(m, -1, &scope, 64) == 0)
        #expect(cnes_scope_get(m, 0, nil, 64) == 0)
        #expect(cnes_scope_get(m, 0, &scope, 0) == 0)
        #expect(cnes_scope_get(m, 0, &scope, 20_000) == 0)
    }

    final class Inside {
        var machine: OpaquePointer?
        var answers: [Int] = []
    }

    @Test func nothingHereWorksFromInsideACallbackAndNullIsSafe() throws {
        let m = try #require(Support.machine(init: Support.note))
        defer { cnes_free(m) }
        let inside = Inside()
        inside.machine = m
        var observer = cnes_observer(
            context: Unmanaged.passUnretained(inside).toOpaque(),
            sound_write: { context, _, _ in
                let inside = Unmanaged<Inside>.fromOpaque(context!).takeUnretainedValue()
                var voices = [cnes_voice](repeating: cnes_voice(), count: 5)
                var scope = [Float](repeating: 0, count: 64)
                inside.answers.append(Int(cnes_voice_count(inside.machine)))
                inside.answers.append(Int(cnes_voices_get(inside.machine, &voices, 5)))
                inside.answers.append(cnes_voice_name(inside.machine, 0) == nil ? 0 : 1)
                inside.answers.append(cnes_scope_get(inside.machine, 0, &scope, 64))
            },
            play_call: nil)
        cnes_set_observer(m, &observer)
        _ = Support.render(m, frames: 100)
        cnes_set_observer(m, nil)
        #expect(!inside.answers.isEmpty && inside.answers.allSatisfy { $0 == 0 })
        #expect(cnes_voice_count(m) == 5)
        #expect(cnes_voice_count(nil) == 0)
        #expect(cnes_voice_name(nil, 0) == nil)
        var voices = [cnes_voice](repeating: cnes_voice(), count: 1)
        #expect(cnes_voices_get(nil, &voices, 1) == 0)
        #expect(cnes_voices_get(m, nil, 1) == 0)
        var scope = [Float](repeating: 0, count: 8)
        #expect(cnes_scope_get(nil, 0, &scope, 8) == 0)
    }
}
