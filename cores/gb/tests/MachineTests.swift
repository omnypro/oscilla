import Testing
import Foundation
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
        #expect(cgb_register(m, 0) == 0x55, "the init routine left its last value in A")
        #expect(cgb_register(m, 9) == cgb_next_pc(m))
        #expect(cgb_next_pc(m) == 0xFFFF, "the init routine has returned")
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

/// What the interface refuses, and what it does with odd arguments.
struct RefusalTests {
    final class Probe {
        var machine: OpaquePointer?
        var snapshot: OpaquePointer?
        var restore: Int32 = -1
        var selected: Int32 = -1
        var channelMask: UInt32 = 0
        var done = false
    }

    @Test func callsInsideACallbackDoNothing() throws {
        let m = try #require(Support.machine(init: Support.store(0x24, 0x44)))
        defer { cgb_free(m) }
        let snapshot = try #require(cgb_snapshot_take(m))
        defer { cgb_snapshot_free(snapshot) }
        let probe = Probe()
        probe.machine = m
        probe.snapshot = snapshot
        var observer = cgb_observer(
            context: Unmanaged.passUnretained(probe).toOpaque(),
            sound_write: { context, _, _ in
                let p = Unmanaged<Probe>.fromOpaque(context!).takeUnretainedValue()
                if p.done { return }
                p.done = true
                cgb_free(p.machine)
                p.restore = cgb_snapshot_restore(p.machine, p.snapshot)
                p.selected = cgb_select_song(p.machine, 1)
                cgb_set_channel_mask(p.machine, 0)
                p.channelMask = cgb_channel_mask(p.machine)
                cgb_set_observer(p.machine, nil)
            },
            play_call: nil)
        cgb_set_observer(m, &observer)
        cgb_run_cycles(m, 1000)
        #expect(probe.done)
        #expect(probe.restore == 3)
        #expect(probe.selected == 0)
        #expect(probe.channelMask == 0, "the mask call returned a neutral value")
        #expect(cgb_channel_mask(m) == 0xF, "the mask did not change")
        #expect(cgb_song(m) == 0)
        cgb_run_cycles(m, 1000)
        #expect(cgb_cycles(m) >= 2000, "the machine is intact and still running")
    }

    @Test func aNullOrEmptyRenderDoesNothing() throws {
        let m = try #require(Support.machine(init: Support.note))
        defer { cgb_free(m) }
        cgb_render_s16(m, nil, 100)
        cgb_render_f32(m, nil, 100)
        var out = [Int16](repeating: 7, count: 4)
        cgb_render_s16(m, &out, 0)
        #expect(out == [7, 7, 7, 7])
        #expect(cgb_cycles(m) == 0)
    }

    @Test func aDirectoryIsNotAGBSFile() throws {
        let dir = try Support.scratchDirectory()
        defer { try? FileManager.default.removeItem(at: dir) }
        var image: OpaquePointer?
        var error = [CChar](repeating: 0, count: 64)
        #expect(cgb_image_load_gbs(dir.path, &image, &error, error.count) == CGB_ERR_IO)
        #expect(image == nil)
        #expect(Support.text(error) == "Not a regular file.")
    }

    @Test func aFileLargerThanTheLimitIsRefused() throws {
        let dir = try Support.scratchDirectory()
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appendingPathComponent("big.gbs")
        var big = Support.gbs(init: [])
        big += [UInt8](repeating: 0, count: 4 << 20)
        try Data(big).write(to: url)
        var image: OpaquePointer?
        #expect(cgb_image_load_gbs(url.path, &image, nil, 0) == CGB_ERR_FORMAT)
        #expect(image == nil)
    }

    @Test func theFirstSongComesFromTheHeader() throws {
        for (header, expected) in [(UInt8(0), 0), (1, 0), (3, 2), (200, 3)] {
            let img = try #require(Support.image(Support.gbs(init: [], first: header)))
            var h = cgb_header()
            cgb_image_header(img, &h)
            #expect(h.first_song == expected, "header value \(header)")
            let m = try #require(cgb_new(img, 48000))
            #expect(cgb_song(m) == expected)
            cgb_free(m)
            cgb_image_free(img)
        }
    }
}
