import Testing
import CGba

/// The state and the scopes of the six channels, through the C interface.
struct VisualizationTests {
    @Test func aSquareChannelReportsItsStateAndTheOthersAreIdle() throws {
        let m = try #require(Support.machine(rom: Support.rom(words: SoundTests.squareProgram)))
        defer { cgba_free(m) }
        var out = [Int16](repeating: 0, count: 2 * 2000)
        cgba_render_s16(m, &out, 2000)
        #expect(cgba_voice_count(m) == 6)
        var voices = [cgba_voice](repeating: cgba_voice(), count: 6)
        #expect(cgba_voices_get(m, &voices, 6) == 6)
        let v = voices[0]
        #expect(v.active == 1 && v.key_on == 1 && v.kind == 0)
        #expect(v.instrument == 2, "a duty of 50%")
        #expect(abs(v.frequency_hz - 128) < 0.001)
        #expect(v.volume == 1 && v.envelope == 1)
        #expect(v.left == 1 && v.right == 1 && v.pan == 0)
        #expect(v.envelope_phase == 0 && v.mask_bit == 0)
        #expect(voices[1].active == 0 && voices[1].key_on == 0 && voices[1].volume == 0)
        #expect(voices[2].envelope == -1 && voices[2].mask_bit == 2)
        #expect(voices[3].kind == 1)
        #expect(voices[4].kind == 2 && voices[4].instrument == -1 && voices[4].mask_bit == 4)
        #expect(String(cString: cgba_voice_name(m, 0)) == "Pulse 1")
        #expect(String(cString: cgba_voice_name(m, 5)) == "Direct B")
        #expect(cgba_voice_name(m, 6) == nil)
        #expect(cgba_voice_name(m, -1) == nil)
        #expect(cgba_voices_get(m, &voices, 3) == 3)
        #expect(cgba_voices_get(m, &voices, 0) == 0)
    }

    @Test func aFifoReportsTheRateOfItsTimerAndShowsItsStream() throws {
        // A square of 64 samples between 100 and -100, which is 512 Hz at
        // the FIFO's 32,768 Hz.
        let buffer: [UInt8] = (0..<8192).map { $0 % 64 < 32 ? 100 : UInt8(bitPattern: -100) }
        let rom = Support.rom(words: MachineTests.fifoProgram) + buffer
        let m = try #require(Support.machine(rom: rom, rate: 44100))
        defer { cgba_free(m) }
        var out = [Int16](repeating: 0, count: 2 * 6000)
        cgba_render_s16(m, &out, 6000)
        var voices = [cgba_voice](repeating: cgba_voice(), count: 6)
        #expect(cgba_voices_get(m, &voices, 6) == 6)
        let a = voices[4]
        #expect(a.active == 1 && a.key_on == 1 && a.kind == 2)
        #expect(abs(a.frequency_hz - 32768) < 0.01)
        #expect(a.volume == 1 && a.left == 1 && a.right == 1 && a.envelope == -1)
        #expect(voices[5].active == 0 && voices[5].volume == 0, "no side hears FIFO B")
        var scope = [Float](repeating: 0, count: 2048)
        #expect(cgba_scope_get(m, 4, &scope, 2048) == 2048)
        #expect(scope.contains { $0 > 0.7 } && scope.contains { $0 < -0.7 })
    }

    @Test func aScopeFollowsTheChannelAndAMutedChannelStillShows() throws {
        let m = try #require(Support.machine(rom: Support.rom(words: SoundTests.squareProgram), rate: 44100))
        defer { cgba_free(m) }
        var out = [Int16](repeating: 0, count: 2 * 8000)
        cgba_render_s16(m, &out, 8000)
        cgba_set_channel_mask(m, 0x3E)
        cgba_render_s16(m, &out, 500)
        var scope = [Float](repeating: 0, count: 4096)
        #expect(cgba_scope_get(m, 0, &scope, 4096) == 4096)
        // 128 Hz over 4,096 frames of 44,100 Hz: 23.8 crossings.
        let crossings = zip(scope, scope.dropFirst()).filter { ($0 < 0) != ($1 < 0) }.count
        #expect((22...25).contains(crossings))
        #expect(scope.contains { $0 > 0.99 } && scope.contains { $0 < -0.99 })
        var idle = [Float](repeating: 1, count: 64)
        #expect(cgba_scope_get(m, 3, &idle, 64) == 64)
        #expect(idle.allSatisfy { $0 == 0 })
        #expect(cgba_scope_get(m, 6, &scope, 64) == 0)
        #expect(cgba_scope_get(m, -1, &scope, 64) == 0)
        #expect(cgba_scope_get(m, 0, nil, 64) == 0)
        #expect(cgba_scope_get(m, 0, &scope, 0) == 0)
        #expect(cgba_scope_get(m, 0, &scope, 20_000) == 0)
    }

    final class Answers {
        var machine: OpaquePointer?
        var calls = 0
        var values: [Int] = []
    }

    @Test func theCallsDoNothingInsideACallbackAndNullIsSafe() throws {
        let buffer = [UInt8](repeating: 1, count: 256)
        let m = try #require(Support.machine(rom: Support.rom(words: MachineTests.fifoProgram) + buffer))
        defer { cgba_free(m) }
        let box = Answers()
        box.machine = m
        var observer = cgba_observer(
            context: Unmanaged.passUnretained(box).toOpaque(),
            sound_write: { ctx, _, _, _ in
                let box = Unmanaged<Answers>.fromOpaque(ctx!).takeUnretainedValue()
                box.calls += 1
                var voices = [cgba_voice](repeating: cgba_voice(), count: 6)
                var scope = [Float](repeating: 0, count: 64)
                box.values.append(Int(cgba_voice_count(box.machine)))
                box.values.append(Int(cgba_voices_get(box.machine, &voices, 6)))
                box.values.append(cgba_voice_name(box.machine, 0) == nil ? 0 : 1)
                box.values.append(cgba_scope_get(box.machine, 0, &scope, 64))
            },
            fifo_sample: nil)
        cgba_set_observer(m, &observer)
        cgba_run_cycles(m, 512 * 100)
        cgba_set_observer(m, nil)
        #expect(box.calls > 0)
        #expect(box.values.allSatisfy { $0 == 0 })
        #expect(cgba_voice_count(m) == 6)
        #expect(cgba_voice_count(nil) == 0)
        #expect(cgba_voices_get(nil, nil, 6) == 0)
        #expect(cgba_voice_name(nil, 0) == nil)
        var scope = [Float](repeating: 0, count: 8)
        #expect(cgba_scope_get(nil, 0, &scope, 8) == 0)
    }
}
