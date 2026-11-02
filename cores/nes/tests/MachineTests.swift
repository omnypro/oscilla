import Testing
import Foundation
import CNes

/// Playing a song through the C interface.
struct MachineTests {
    @Test func aNoteSoundsAndTheSameSongRendersTheSameSamples() throws {
        let a = try #require(Support.machine(init: Support.note))
        let b = try #require(Support.machine(init: Support.note))
        defer { cnes_free(a); cnes_free(b) }
        let first = Support.render(a, frames: 24000)
        #expect(first == Support.render(b, frames: 24000))
        #expect(first.contains { abs(Int($0)) > 3000 })
        cnes_reset(a)
        #expect(Support.render(a, frames: 24000) == first)
    }

    @Test func theFloatOutputFollowsTheIntegerOutput() throws {
        let a = try #require(Support.machine(init: Support.note))
        let b = try #require(Support.machine(init: Support.note))
        defer { cnes_free(a); cnes_free(b) }
        let ints = Support.render(a, frames: 4800)
        var floats = [Float](repeating: 0, count: 9600)
        cnes_render_f32(b, &floats, 4800)
        for (i, f) in zip(ints, floats) {
            #expect(abs(Float(i) / 32768 - f) < 0.001)
        }
    }

    @Test func theRateIsChecked() throws {
        let img = try #require(Support.image(Support.nsf(init: [])))
        defer { cnes_image_free(img) }
        #expect(cnes_new(img, 7999) == nil)
        #expect(cnes_new(img, 192001) == nil)
        #expect(cnes_new_for(img, 7999, CNES_NTSC) == nil)
        #expect(cnes_new_for(img, 192001, CNES_PAL) == nil)
        let m = try #require(cnes_new(img, 8000))
        cnes_free(m)
    }

    @Test func aSongIsSelectedByItsIndex() throws {
        let m = try #require(Support.machine(init: [0x8D, 0x01, 0x02]))
        defer { cnes_free(m) }
        #expect(cnes_song(m) == 0)
        #expect(cnes_select_song(m, 2) == 1)
        #expect(cnes_song(m) == 2)
        cnes_run_cycles(m, 1000)
        #expect(cnes_peek(m, 0x0201) == 2, "the init routine got the song number in A")
        #expect(cnes_select_song(m, 4) == 0)
        #expect(cnes_select_song(m, -1) == 0)
        #expect(cnes_song(m) == 2)
    }

    @Test func theRegionComesFromTheFileOrFromTheCaller() throws {
        let cases: [(bits: UInt8, expected: Int32)] = [
            (0, Int32(CNES_NTSC.rawValue)), (1, Int32(CNES_PAL.rawValue)), (2, Int32(CNES_NTSC.rawValue)),
        ]
        for (bits, expected) in cases {
            let m = try #require(Support.machine(init: [], region: bits))
            #expect(cnes_region_get(m) == expected, "region byte \(bits)")
            cnes_free(m)
        }
        let img = try #require(Support.image(Support.nsf(init: [], region: 1)))
        defer { cnes_image_free(img) }
        let ntsc = try #require(cnes_new_for(img, 48000, CNES_NTSC))
        let pal = try #require(cnes_new_for(img, 48000, CNES_PAL))
        defer { cnes_free(ntsc); cnes_free(pal) }
        #expect(cnes_region_get(ntsc) == Int32(CNES_NTSC.rawValue))
        #expect(cnes_region_get(pal) == Int32(CNES_PAL.rawValue))
        #expect(cnes_new_for(img, 48000, cnes_region(rawValue: 2)) == nil)
        #expect(cnes_new_for(img, 48000, cnes_region(rawValue: UInt32.max)) == nil)
    }

    @Test func theInitRoutineGetsTheRegionInX() throws {
        let ntsc = try #require(Support.machine(init: [0x8E, 0x00, 0x02], region: CNES_NTSC))
        let pal = try #require(Support.machine(init: [0x8E, 0x00, 0x02], region: CNES_PAL))
        defer { cnes_free(ntsc); cnes_free(pal) }
        cnes_run_cycles(ntsc, 1000)
        cnes_run_cycles(pal, 1000)
        #expect(cnes_peek(ntsc, 0x0200) == 0)
        #expect(cnes_peek(pal, 0x0200) == 1)
    }

    @Test func theRegionSetsTheClockAndThePlayInterval() throws {
        let ntsc = try #require(Support.machine(init: [], play: Support.store(0x0200, 1), region: CNES_NTSC))
        let pal = try #require(Support.machine(init: [], play: Support.store(0x0200, 1), region: CNES_PAL))
        defer { cnes_free(ntsc); cnes_free(pal) }
        cnes_run_cycles(ntsc, UInt64(CNES_NTSC_CLOCK_HZ))
        cnes_run_cycles(pal, UInt64(CNES_PAL_CLOCK_HZ))
        #expect((59...61).contains(cnes_frames(ntsc)), "60 per second at 16639 microseconds")
        #expect((49...51).contains(cnes_frames(pal)), "50 per second at 19997 microseconds")
    }

    @Test func theCycleAndFrameCountersAdvance() throws {
        let m = try #require(Support.machine(init: [], play: Support.store(0x0200, 0x44)))
        defer { cnes_free(m) }
        let half = UInt64(CNES_NTSC_CLOCK_HZ / 2)
        cnes_run_cycles(m, half)
        #expect(cnes_cycles(m) >= half)
        #expect((29...31).contains(cnes_frames(m)))
    }

    @Test func registersAndMemoryAreReadable() throws {
        let m = try #require(Support.machine(init: Support.store(0x0200, 0x55)))
        defer { cnes_free(m) }
        cnes_run_cycles(m, 1000)
        #expect(cnes_register(m, 0) == 0x55, "the init routine left its last value in A")
        #expect(cnes_register(m, 5) == cnes_next_pc(m))
        #expect(cnes_next_pc(m) == 0xFFFF, "the init routine has returned")
        #expect(cnes_peek(m, 0x0200) == 0x55)
        #expect(cnes_peek(m, 0x0A00) == 0x55, "the RAM repeats every 2 KiB")
        #expect(cnes_peek(m, 0x10000) == 0)
        #expect(cnes_register(m, 99) == 0)
        #expect(cnes_register(m, -1) == 0)
    }

    @Test func theStatusRegisterShowsTheChannelsThatSound() throws {
        let m = try #require(Support.machine(init: Support.note))
        defer { cnes_free(m) }
        cnes_run_cycles(m, 1000)
        #expect(cnes_peek(m, 0x4015) & 0x1F == 0x01, "only pulse 1 has a length")
    }

    @Test func aRoutineThatNeverReturnsIsReported() throws {
        let m = try #require(Support.machine(init: [], play: [0x4C, 0x00, 0x81]))
        defer { cnes_free(m) }
        cnes_run_cycles(m, UInt64(CNES_NTSC_CLOCK_HZ) * 5 / 4)
        #expect(cnes_end_state_get(m) == CNES_STALLED)
        #expect(String(cString: cnes_warnings(m)).contains("did not return"))
    }

    @Test func anOpcodeThatLocksTheCPUIsReported() throws {
        let m = try #require(Support.machine(init: [], play: [0x02]))
        defer { cnes_free(m) }
        cnes_run_cycles(m, UInt64(CNES_NTSC_CLOCK_HZ) / 10)
        #expect(cnes_end_state_get(m) == CNES_STALLED)
        #expect(String(cString: cnes_warnings(m)).contains("locks"))
    }

    @Test func expansionSoundIsReported() throws {
        let plain = try #require(Support.machine(init: Support.note))
        let vrc6 = try #require(Support.machine(init: Support.note, chips: 1))
        let other = try #require(Support.machine(init: Support.note, chips: 4))
        defer { cnes_free(plain); cnes_free(vrc6); cnes_free(other) }
        #expect(String(cString: cnes_warnings(plain)).isEmpty)
        #expect(String(cString: cnes_warnings(vrc6)).isEmpty, "the VRC6 is played")
        #expect(String(cString: cnes_warnings(other)).contains("expansion"))
    }

    @Test func theEndStateTellsAnEmptyUnitFromAPlayingOne() throws {
        let silent = try #require(Support.machine(init: Support.power))
        let playing = try #require(Support.machine(init: Support.note))
        defer { cnes_free(silent); cnes_free(playing) }
        let tenth = UInt64(CNES_NTSC_CLOCK_HZ / 10)
        cnes_run_cycles(silent, tenth)
        cnes_run_cycles(playing, tenth)
        #expect(cnes_end_state_get(silent) == CNES_CHANNELS_OFF)
        #expect(cnes_end_state_get(playing) == CNES_RUNNING)
        #expect(cnes_quiet_cycles(silent) > tenth / 2)
        #expect(cnes_quiet_cycles(playing) < tenth / 10)
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
        let m = try #require(Support.machine(init: Support.store(0x4015, 0x0F)))
        defer { cnes_free(m) }
        let snapshot = try #require(cnes_snapshot_take(m))
        defer { cnes_snapshot_free(snapshot) }
        let probe = Probe()
        probe.machine = m
        probe.snapshot = snapshot
        var observer = cnes_observer(
            context: Unmanaged.passUnretained(probe).toOpaque(),
            sound_write: { context, _, _ in
                let p = Unmanaged<Probe>.fromOpaque(context!).takeUnretainedValue()
                if p.done { return }
                p.done = true
                cnes_free(p.machine)
                p.restore = cnes_snapshot_restore(p.machine, p.snapshot)
                p.selected = cnes_select_song(p.machine, 1)
                cnes_set_channel_mask(p.machine, 0)
                p.channelMask = cnes_channel_mask(p.machine)
                cnes_set_observer(p.machine, nil)
            },
            play_call: nil)
        cnes_set_observer(m, &observer)
        cnes_run_cycles(m, 1000)
        #expect(probe.done)
        #expect(probe.restore == 3)
        #expect(probe.selected == 0)
        #expect(probe.channelMask == 0, "the mask call returned a neutral value")
        #expect(cnes_channel_mask(m) == 0xFF, "the mask did not change")
        #expect(cnes_song(m) == 0)
        cnes_run_cycles(m, 1000)
        #expect(cnes_cycles(m) >= 2000, "the machine is intact and still running")
    }

    @Test func aNullOrEmptyRenderDoesNothing() throws {
        let m = try #require(Support.machine(init: Support.note))
        defer { cnes_free(m) }
        cnes_render_s16(m, nil, 100)
        cnes_render_f32(m, nil, 100)
        var out = [Int16](repeating: 7, count: 4)
        cnes_render_s16(m, &out, 0)
        #expect(out == [7, 7, 7, 7])
        #expect(cnes_cycles(m) == 0)
    }

    @Test func aDirectoryIsNotAnNSFFile() throws {
        let dir = try Support.scratchDirectory()
        defer { try? FileManager.default.removeItem(at: dir) }
        var image: OpaquePointer?
        var error = [CChar](repeating: 0, count: 64)
        #expect(cnes_image_load_nsf(dir.path, &image, &error, error.count) == CNES_ERR_IO)
        #expect(image == nil)
        #expect(Support.text(error) == "Not a regular file.")
    }

    @Test func aFileLargerThanTheLimitIsRefused() throws {
        let dir = try Support.scratchDirectory()
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appendingPathComponent("big.nsf")
        var big = Support.nsf(init: [])
        big += [UInt8](repeating: 0, count: 4 << 20)
        try Data(big).write(to: url)
        var image: OpaquePointer?
        #expect(cnes_image_load_nsf(url.path, &image, nil, 0) == CNES_ERR_FORMAT)
        #expect(image == nil)
    }

    @Test func theFirstSongComesFromTheHeader() throws {
        for (header, expected) in [(UInt8(0), 0), (1, 0), (3, 2), (200, 3)] {
            let img = try #require(Support.image(Support.nsf(init: [], first: header)))
            var h = cnes_header()
            cnes_image_header(img, &h)
            #expect(h.first_song == expected, "header value \(header)")
            let m = try #require(cnes_new(img, 48000))
            #expect(cnes_song(m) == expected)
            cnes_free(m)
            cnes_image_free(img)
        }
    }
}
