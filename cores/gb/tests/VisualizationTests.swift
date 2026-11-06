import Testing
import CGb

/// The state of each channel and its scope, through the C interface.
@Suite("Visualization")
struct VisualizationTests {
    final class Log {
        var machine: OpaquePointer?
        /// What the voice calls answered from inside a callback.
        var inside: [Int] = []
        var names: [UnsafePointer<CChar>?] = []
    }

    @Test func aPlayingChannelReportsItsStateAndTheOthersAreIdle() throws {
        let m = try #require(Support.machine(init: Support.note, rate: 44100))
        defer { cgb_free(m) }
        _ = Support.render(m, frames: 4000)
        #expect(cgb_voice_count(m) == 4)
        var voices = [cgb_voice](repeating: cgb_voice(), count: 4)
        #expect(cgb_voices_get(m, &voices, 4) == 4)
        let v = voices[1]
        #expect(v.active == 1 && v.key_on == 1 && v.kind == 0)
        #expect(abs(v.frequency_hz - 512) < 0.01)
        #expect(v.volume == 1 && v.envelope == 1)
        #expect(v.left == 1 && v.right == 1 && abs(v.pan) < 0.001)
        #expect(v.instrument == 2, "NR21 0x80 is duty 2")
        #expect(v.mask_bit == 1 && voices[3].mask_bit == 3)
        #expect(voices[0].active == 0 && voices[0].key_on == 0 && voices[0].volume == 0)
        #expect(voices[2].envelope == -1)
        #expect(voices[3].kind == 1 && voices[3].frequency_hz == 0)
        #expect(String(cString: cgb_voice_name(m, 0)) == "Pulse 1")
        #expect(String(cString: cgb_voice_name(m, 3)) == "Noise")
        #expect(cgb_voice_name(m, 4) == nil)
        #expect(cgb_voice_name(m, -1) == nil)
        #expect(cgb_voices_get(m, &voices, 2) == 2)
        #expect(cgb_voices_get(m, &voices, 0) == 0)
        #expect(cgb_voices_get(m, nil, 4) == 0)
    }

    @Test func aScopeHoldsTheChannelAndAMutedChannelStillShows() throws {
        let m = try #require(Support.machine(init: Support.note, rate: 44100))
        defer { cgb_free(m) }
        _ = Support.render(m, frames: 8000)
        cgb_set_channel_mask(m, 0)
        _ = Support.render(m, frames: 500)
        var scope = [Float](repeating: 0, count: 2048)
        #expect(cgb_scope_get(m, 1, &scope, 2048) == 2048)
        #expect(scope.contains { $0 > 0.99 } && scope.contains { $0 < -0.99 })
        var idle = [Float](repeating: 1, count: 64)
        #expect(cgb_scope_get(m, 0, &idle, 64) == 64)
        #expect(idle.allSatisfy { $0 == 0 })
        #expect(cgb_scope_get(m, 4, &scope, 64) == 0)
        #expect(cgb_scope_get(m, -1, &scope, 64) == 0)
        #expect(cgb_scope_get(m, 1, nil, 64) == 0)
        #expect(cgb_scope_get(m, 1, &scope, 0) == 0)
        #expect(cgb_scope_get(m, 1, &scope, 20_000) == 0)
    }

    @Test func nothingHereWorksFromInsideACallbackAndNullIsSafe() throws {
        let m = try #require(Support.machine(init: Support.note))
        defer { cgb_free(m) }
        let log = Log()
        log.machine = m
        var observer = cgb_observer(
            context: Unmanaged.passUnretained(log).toOpaque(),
            sound_write: { context, _, _ in
                let log = Unmanaged<Log>.fromOpaque(context!).takeUnretainedValue()
                var voices = [cgb_voice](repeating: cgb_voice(), count: 4)
                var scope = [Float](repeating: 0, count: 8)
                log.inside.append(Int(cgb_voice_count(log.machine)))
                log.inside.append(Int(cgb_voices_get(log.machine, &voices, 4)))
                log.inside.append(cgb_scope_get(log.machine, 1, &scope, 8))
                log.names.append(cgb_voice_name(log.machine, 0))
            },
            play_call: nil)
        cgb_set_observer(m, &observer)
        _ = Support.render(m, frames: 1000)
        cgb_set_observer(m, nil)
        #expect(!log.inside.isEmpty)
        #expect(log.inside.allSatisfy { $0 == 0 })
        #expect(log.names.allSatisfy { $0 == nil })
        #expect(cgb_voice_count(m) == 4, "outside the callback the calls answer again")
        #expect(cgb_voice_count(nil) == 0)
        #expect(cgb_voice_name(nil, 0) == nil)
        var voices = [cgb_voice](repeating: cgb_voice(), count: 4)
        #expect(cgb_voices_get(nil, &voices, 4) == 0)
        var scope = [Float](repeating: 0, count: 8)
        #expect(cgb_scope_get(nil, 0, &scope, 8) == 0)
    }
}
