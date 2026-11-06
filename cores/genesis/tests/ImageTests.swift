import Foundation
import Testing
import CGenesis

@Suite("Images")
struct ImageTests {
    @Test func aFileLoadsAndShowsItsTagsAndTimes() {
        let bytes = Support.vgm(stream: Support.wait(100), loopAt: 0, total: 44_100 * 3,
                                loopSamples: 44_100 * 2,
                                tag: ["Theme", "テーマ", "Game", "ゲーム", "Genesis", "メガドライブ",
                                      "Composer", "作曲者", "1991/06/23", "Ripper", "Notes"])
        let image = Support.image(bytes)
        defer { cgenesis_image_free(image) }
        #expect(String(cString: cgenesis_image_title(image)) == "Theme")
        #expect(String(cString: cgenesis_image_title_jp(image)) == "テーマ")
        #expect(String(cString: cgenesis_image_game(image)) == "Game")
        #expect(String(cString: cgenesis_image_system(image)) == "Genesis")
        #expect(String(cString: cgenesis_image_author(image)) == "Composer")
        #expect(String(cString: cgenesis_image_date(image)) == "1991/06/23")
        #expect(String(cString: cgenesis_image_ripper(image)) == "Ripper")
        #expect(String(cString: cgenesis_image_notes(image)) == "Notes")
        #expect(cgenesis_image_chips(image) == 3)
        #expect(cgenesis_image_has_loop(image) == 1)
        #expect(cgenesis_image_total_samples(image) == 132_300)
        #expect(cgenesis_image_loop_samples(image) == 88_200)
        #expect(cgenesis_image_intro_samples(image) == 44_100)
        #expect(cgenesis_image_intro_frames(image, 48_000) == 48_000)
        #expect(cgenesis_image_loop_frames(image, 48_000) == 96_000)
        #expect(cgenesis_image_loop_frames(image, 0) == -1)
    }

    @Test func aFileWithoutALoopOrATag() {
        let image = Support.image(Support.vgm(stream: Support.wait(100), psg: true, fm: false,
                                              total: 100))
        defer { cgenesis_image_free(image) }
        #expect(String(cString: cgenesis_image_title(image)) == "")
        #expect(cgenesis_image_chips(image) == 1)
        #expect(cgenesis_image_has_loop(image) == 0)
        #expect(cgenesis_image_loop_frames(image, 44_100) == 0)
        #expect(cgenesis_image_intro_samples(image) == 100)
        #expect(cgenesis_image_loop_count(image, 2) == 2)
    }

    @Test func badInputIsRefusedWithASentence() {
        var image: OpaquePointer?
        var error = [CChar](repeating: 0, count: 256)
        let junk: [UInt8] = Array("not a vgm file at all".utf8)
        let status = junk.withUnsafeBufferPointer {
            cgenesis_image_from_vgm($0.baseAddress, $0.count, &image, &error, error.count)
        }
        #expect(status == CGENESIS_ERR_FORMAT)
        #expect(image == nil)
        #expect(error[0] != 0)
        #expect(cgenesis_image_load_vgm("/nonexistent/file.vgm", &image, &error, error.count)
                == CGENESIS_ERR_IO)
    }

    @Test func aGymLogLoadsAsAnImageAndPlays() {
        var header = [UInt8](repeating: 0, count: 428)
        header.replaceSubrange(0..<4, with: Array("GYMX".utf8))
        header.replaceSubrange(4..<8, with: Array("Song".utf8))
        header[420] = 1
        var log: [UInt8] = []
        for step in 0..<4 as Range<UInt8> {
            log += [1, 0x30 + 4 * step, 0x01, 1, 0x40 + 4 * step, step == 0 ? 0 : 127]
            log += [1, 0x50 + 4 * step, 0x1F, 1, 0x60 + 4 * step, 0, 1, 0x70 + 4 * step, 0]
            log += [1, 0x80 + 4 * step, 0x0F]
        }
        log += [1, 0xB0, 7, 1, 0xB4, 0xC0, 1, 0xA4, 0x22, 1, 0xA0, 0x4B, 1, 0x28, 0xF0]
        log += [UInt8](repeating: 0, count: 30)
        let bytes = header + log
        var image: OpaquePointer?
        var error = [CChar](repeating: 0, count: 256)
        let status = bytes.withUnsafeBufferPointer {
            cgenesis_image_from_gym($0.baseAddress, $0.count, 0, &image, &error, error.count)
        }
        #expect(status == CGENESIS_OK)
        defer { cgenesis_image_free(image) }
        #expect(String(cString: cgenesis_image_title(image)) == "Song")
        #expect(cgenesis_image_total_samples(image) == 30 * 735)
        #expect(cgenesis_image_loop_samples(image) == 29 * 735)
        #expect(cgenesis_image_frame_rate(image) == 60)
        let machine = cgenesis_new(image, 44_100)!
        defer { cgenesis_free(machine) }
        #expect(Support.rms(Support.render(machine, frames: 20 * 735)) > 0.01)
        var pal: OpaquePointer?
        let again = bytes.withUnsafeBufferPointer {
            cgenesis_image_from_gym($0.baseAddress, $0.count, 1, &pal, &error, error.count)
        }
        #expect(again == CGENESIS_OK)
        #expect(cgenesis_image_frame_rate(pal) == 50)
        cgenesis_image_free(pal)
    }

    @Test func aBadGymLogIsRefused() {
        var image: OpaquePointer?
        var error = [CChar](repeating: 0, count: 256)
        let junk: [UInt8] = [0, 0, 9]
        let status = junk.withUnsafeBufferPointer {
            cgenesis_image_from_gym($0.baseAddress, $0.count, 0, &image, &error, error.count)
        }
        #expect(status == CGENESIS_ERR_FORMAT)
        #expect(image == nil)
        #expect(String(cString: error) == "Not a GYM command at byte 2.")
    }
}
