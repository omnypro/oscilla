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
}
