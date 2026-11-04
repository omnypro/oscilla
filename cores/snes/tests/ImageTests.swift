import Foundation
import Testing
import CSnes

@Suite("Images")
struct ImageTests {
    @Test func aFileLoadsAndShowsItsTags() {
        var bytes = Support.spc(program: [0x2F, 0xFE])
        bytes.replaceSubrange(0x2E..<0x33, with: Array("Theme".utf8))
        bytes.replaceSubrange(0x4E..<0x52, with: Array("Game".utf8))
        bytes.replaceSubrange(0x9E..<0xA8, with: Array("11/03/2026".utf8))
        bytes.replaceSubrange(0xA9..<0xAC, with: Array("120".utf8))
        bytes.replaceSubrange(0xAC..<0xB0, with: Array("4000".utf8))
        let image = Support.image(bytes)
        defer { csnes_image_free(image) }
        #expect(String(cString: csnes_image_song(image)) == "Theme")
        #expect(String(cString: csnes_image_game(image)) == "Game")
        #expect(csnes_image_time_ms(image) == 120_000)
        #expect(csnes_image_fade_ms(image) == 4000)
        #expect(csnes_image_length_ms(image) == 124_000)
    }

    @Test func aFileWithoutATagHasNoTimes() {
        let image = Support.image(Support.spc(program: [0x2F, 0xFE], tagged: false))
        defer { csnes_image_free(image) }
        #expect(String(cString: csnes_image_song(image)) == "")
        #expect(csnes_image_time_ms(image) == -1)
        #expect(csnes_image_length_ms(image) == -1)
        #expect(csnes_image_amplification(image) == 65536)
        #expect(csnes_image_muted_voices(image) == 0)
    }

    @Test func theExtendedTagsWinOverTheOldOnes() {
        var bytes = Support.spc(program: [0x2F, 0xFE])
        // Intro 1 s, loop 2 s, end 0.5 s, two loops, amplification 2.0.
        var items: [UInt8] = []
        func integer(_ id: UInt8, _ value: Int32) {
            items += [id, 4, 4, 0] + withUnsafeBytes(of: value.littleEndian) { Array($0) }
        }
        integer(0x30, 64_000)
        integer(0x31, 128_000)
        integer(0x32, 32_000)
        items += [0x35, 0, 2, 0]
        items += [0x34, 0, 0x0C, 0]
        integer(0x36, 131_072)
        bytes += Array("xid6".utf8) + withUnsafeBytes(of: UInt32(items.count).littleEndian) { Array($0) } + items
        let image = Support.image(bytes)
        defer { csnes_image_free(image) }
        #expect(csnes_image_time_ms(image) == 5500)
        #expect(csnes_image_muted_voices(image) == 0x0C)
        #expect(csnes_image_amplification(image) == 131_072)
    }

    @Test func badFilesAreRefusedWithAnError() {
        var image: OpaquePointer?
        var error = [CChar](repeating: 0, count: 256)
        let bytes: [UInt8] = Array("NOPE".utf8)
        let status = bytes.withUnsafeBufferPointer {
            csnes_image_from_spc($0.baseAddress, $0.count, &image, &error, error.count)
        }
        #expect(status == CSNES_ERR_FORMAT)
        #expect(image == nil)
        #expect(String(cString: error) == "Not an SPC file.")
        #expect(csnes_image_load_spc("/nonexistent/x.spc", &image, &error, error.count) == CSNES_ERR_IO)
    }

    @Test func aFileLoadsFromAPath() throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("oscilla-\(UUID().uuidString).spc")
        try Data(Support.tone()).write(to: url)
        defer { try? FileManager.default.removeItem(at: url) }
        var image: OpaquePointer?
        var error = [CChar](repeating: 0, count: 256)
        #expect(csnes_image_load_spc(url.path, &image, &error, error.count) == CSNES_OK)
        csnes_image_free(image)
    }
}
