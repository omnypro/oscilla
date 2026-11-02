import Foundation
import Testing
import CGba

/// The sound unit's levels and the sound DMA's pacing. Both are pinned to the
/// behavior that the reference run showed.
struct SoundTests {
    /// Sound on, channel 1 to both sides at master volume 7 and PSG volume
    /// 100%, a 50% duty square at volume 15 and 128 Hz, then park.
    static let squareProgram: [UInt32] = [
        0xE3A0_0301, 0xE3A0_1080, 0xE1C0_18B4, 0xE3A0_1C11, 0xE381_1077, 0xE1C0_18B0,
        0xE3A0_1002, 0xE1C0_18B2, 0xE3A0_1A0F, 0xE381_1080, 0xE1C0_16B2, 0xE3A0_1902,
        0xE381_1B01, 0xE1C0_16B4, 0xEAFF_FFFE,
    ]

    @Test func aSquareChannelIsUnipolar() throws {
        let m = try #require(Support.machine(rom: Support.rom(words: Self.squareProgram)))
        defer { cgba_free(m) }
        var out = [Int16](repeating: 0, count: 2 * 16384)
        cgba_render_s16(m, &out, 16384)
        let left = stride(from: 8192, to: out.count, by: 2).map { Int(out[$0]) }
        // Volume 15 is 240 units of the 10-bit output, 48 output steps each
        // (the 10-bit range fills three quarters of the 16-bit one): the
        // square's halves sit at 0 and 11520, with band-limited ripple after
        // each edge, so each half is read by its median.
        let lows = left.filter { $0 < 5760 }.sorted(), highs = left.filter { $0 >= 5760 }.sorted()
        let low = lows[lows.count / 2], high = highs[highs.count / 2]
        #expect(abs(low) < 48, "the low half of the square sits at zero")
        #expect(abs(high - 11520) < 48, "the high half is volume 15 at full mix")
        #expect(abs(lows.count - highs.count) < left.count / 20, "the duty is 50%")
    }

    @Test func floatOutputHasTheSameScaleAsSixteenBit() throws {
        let rom = Support.rom(words: Self.squareProgram)
        let a = try #require(Support.machine(rom: rom)), b = try #require(Support.machine(rom: rom))
        defer { cgba_free(a); cgba_free(b) }
        var ints = [Int16](repeating: 0, count: 2 * 16384)
        var floats = [Float](repeating: 0, count: 2 * 16384)
        cgba_render_s16(a, &ints, 16384)
        cgba_render_f32(b, &floats, 16384)
        let highs = stride(from: 8192, to: floats.count, by: 2).map { floats[$0] }.filter { $0 >= 0.176 }.sorted()
        let high = highs[highs.count / 2]
        #expect(abs(Double(high) - 11520.0 / 32768.0) < 0.002, "volume 15 is 240 units of 48 steps, of 32768")
        let worst = zip(ints, floats).map { abs(Double($0) / 32768.0 - Double($1)) }.max() ?? 1
        #expect(worst < 1.0 / 32768.0, "both formats come from the same samples")
    }

    final class Counts { var pops = 0; var words = 0 }

    @Test func soundDmaRefillsWhenFewerThanSixteenBytesRemain() throws {
        let buffer: [UInt8] = (0..<4096).map { UInt8(truncatingIfNeeded: $0 &* 5) }
        let m = try #require(Support.machine(rom: Support.rom(words: MachineTests.fifoProgram) + buffer))
        defer { cgba_free(m) }
        let counts = Counts()
        var observer = cgba_observer(
            context: Unmanaged.passUnretained(counts).toOpaque(),
            sound_write: { ctx, address, _, width in
                if address == 0x0400_00A0 && width == 4 {
                    Unmanaged<Counts>.fromOpaque(ctx!).takeUnretainedValue().words += 1
                }
            },
            fifo_sample: { ctx, channel, _ in
                if channel == 0 { Unmanaged<Counts>.fromOpaque(ctx!).takeUnretainedValue().pops += 1 }
            })
        cgba_set_observer(m, &observer)
        for _ in 0..<200 {
            cgba_run_cycles(m, 512 * 7 + 13)
            let queued = counts.words * 4 - counts.pops
            #expect((15...31).contains(queued), "the FIFO holds \(queued) bytes")
        }
        cgba_set_observer(m, nil)
    }
}

/// A real GSF, when GSF_TEST_FILE names one (a .minigsf beside its
/// library). Skipped otherwise: game rips are not part of the repository.
struct CorpusTests {
    static let file: String? = ProcessInfo.processInfo.environment["GSF_TEST_FILE"]

    @Test(.enabled(if: file != nil, "set GSF_TEST_FILE to a .minigsf"))
    func aGsfPlaysAndRepeatsItself() throws {
        var image: OpaquePointer?
        var error = [CChar](repeating: 0, count: 256)
        guard let file = Self.file else { return }
        let status = cgba_image_load_gsf(file, &image, &error, error.count)
        #expect(status == CGBA_OK, "\(Support.text(error))")
        let img = try #require(image)
        defer { cgba_image_free(img) }
        let a = try #require(cgba_new(img, 48000))
        let b = try #require(cgba_new(img, 48000))
        defer { cgba_free(a); cgba_free(b) }
        var first = [Int16](repeating: 0, count: 2 * 48000 * 5)
        var second = first
        cgba_render_s16(a, &first, 48000 * 5)
        cgba_render_s16(b, &second, 48000 * 5)
        #expect(first == second, "two machines on one image play the same samples")
        let tail = first.suffix(2 * 48000 * 2)
        #expect(tail.contains { abs(Int($0) - Int(tail.first!)) > 1000 }, "the song is audible after boot")
    }
}
