import Testing
import CGb

/// Playing a song through the C interface.
struct MachineTests {
    @Test func aNoteSoundsAndTheSameSongRendersTheSameSamples() throws {
        let a = try #require(Support.machine(init: Support.note))
        let b = try #require(Support.machine(init: Support.note))
        defer { cgb_free(a); cgb_free(b) }
        let first = Support.render(a, frames: 24000)
        #expect(first == Support.render(b, frames: 24000))
        #expect(first.contains { abs(Int($0)) > 3000 })
        cgb_reset(a)
        #expect(Support.render(a, frames: 24000) == first)
    }

    @Test func theRateIsChecked() throws {
        let img = try #require(Support.image(Support.gbs(init: [])))
        defer { cgb_image_free(img) }
        #expect(cgb_new(img, 7999) == nil)
        #expect(cgb_new(img, 192001) == nil)
        let m = try #require(cgb_new(img, 8000))
        cgb_free(m)
    }

    @Test func aSongIsSelectedByItsIndex() throws {
        let m = try #require(Support.machine(init: [0xE0, 0x24]))
        defer { cgb_free(m) }
        #expect(cgb_song(m) == 0)
        #expect(cgb_select_song(m, 2) == 1)
        #expect(cgb_song(m) == 2)
        cgb_run_cycles(m, 1000)
        #expect(cgb_peek(m, 0xFF24) == 2)
        #expect(cgb_select_song(m, 4) == 0)
        #expect(cgb_select_song(m, -1) == 0)
        #expect(cgb_song(m) == 2)
    }

    @Test func theCycleAndFrameCountersAdvance() throws {
        let m = try #require(Support.machine(init: [], play: Support.store(0x24, 0x44)))
        defer { cgb_free(m) }
        cgb_run_cycles(m, UInt64(CGB_CLOCK_HZ) / 2)
        #expect(cgb_cycles(m) >= UInt64(CGB_CLOCK_HZ) / 2)
        let frames = cgb_frames(m)
        #expect((29...31).contains(frames))
    }

    @Test func registersAndMemoryAreReadable() throws {
        let m = try #require(Support.machine(init: Support.store(0x24, 0x55)))
        defer { cgb_free(m) }
        cgb_run_cycles(m, 1000)
        #expect(cgb_register(m, 9) == cgb_next_pc(m))
        #expect(cgb_peek(m, 0xFF24) == 0x55, "the unit starts on, so the store lands")
        #expect(cgb_peek(m, 0x10000) == 0)
        #expect(cgb_register(m, 99) == 0)
    }

    @Test func aRoutineThatNeverReturnsIsReported() throws {
        let m = try #require(Support.machine(init: [], play: [0x18, 0xFE]))
        defer { cgb_free(m) }
        cgb_run_cycles(m, UInt64(CGB_CLOCK_HZ) * 5 / 4)
        #expect(cgb_end_state_get(m) == CGB_STALLED)
        #expect(String(cString: cgb_warnings(m)).contains("did not return"))
    }

    @Test func theEndStateTellsAnEmptyUnitFromAPlayingOne() throws {
        let silent = try #require(Support.machine(init: Support.power))
        let playing = try #require(Support.machine(init: Support.note))
        defer { cgb_free(silent); cgb_free(playing) }
        cgb_run_cycles(silent, UInt64(CGB_CLOCK_HZ) / 10)
        cgb_run_cycles(playing, UInt64(CGB_CLOCK_HZ) / 10)
        #expect(cgb_end_state_get(silent) == CGB_CHANNELS_OFF)
        #expect(cgb_end_state_get(playing) == CGB_RUNNING)
        #expect(cgb_quiet_cycles(silent) > UInt64(CGB_CLOCK_HZ) / 20)
        #expect(cgb_quiet_cycles(playing) < UInt64(CGB_CLOCK_HZ) / 100)
    }
}
