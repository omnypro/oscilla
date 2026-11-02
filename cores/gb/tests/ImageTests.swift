import Foundation
import Testing
import CGb

/// Parsing a GBS file through the C interface.
struct ImageTests {
    @Test func theHeaderComesBack() throws {
        let img = try #require(Support.image(Support.gbs(init: [], songs: 7, tma: 0xBB, tac: 0x04)))
        defer { cgb_image_free(img) }
        var h = cgb_header()
        cgb_image_header(img, &h)
        #expect(h.songs == 7)
        #expect(h.first_song == 0)
        #expect(h.load == 0x0400)
        #expect(h.play == 0x0600)
        #expect(h.sp == 0xDFFF)
        #expect(h.tma == 0xBB)
        #expect(h.tac == 0x04)
        #expect(String(cString: cgb_image_title(img)) == "Title")
        #expect(String(cString: cgb_image_author(img)) == "Author")
        #expect(String(cString: cgb_image_copyright(img)) == "?")
        #expect(cgb_image_tick_clocks(img) == 69 * 1024)
    }

    @Test func aFileMustBeAGBSFile() {
        var image: OpaquePointer?
        var error = [CChar](repeating: 0, count: 128)
        #expect(cgb_image_from_gbs([1, 2, 3], 3, &image, &error, error.count) == CGB_ERR_FORMAT)
        #expect(image == nil)
        #expect(Support.text(error) == "Not a GBS file.")

        var bad = Support.gbs(init: [])
        bad[3] = 2
        #expect(cgb_image_from_gbs(bad, bad.count, &image, &error, error.count) == CGB_ERR_FORMAT)
        #expect(Support.text(error) == "GBS version 2 is not supported.")

        var none = Support.gbs(init: [])
        none[4] = 0
        #expect(cgb_image_from_gbs(none, none.count, &image, &error, error.count) == CGB_ERR_FORMAT)

        var high = Support.gbs(init: [])
        high[7] = 0x80
        #expect(cgb_image_from_gbs(high, high.count, &image, &error, error.count) == CGB_ERR_FORMAT)
        #expect(image == nil)
    }

    @Test func aMissingFileIsAnIOError() {
        var image: OpaquePointer?
        var error = [CChar](repeating: 0, count: 128)
        #expect(cgb_image_load_gbs("/nonexistent/file.gbs", &image, &error, error.count) == CGB_ERR_IO)
        #expect(image == nil)
        #expect(!Support.text(error).isEmpty)
    }

    @Test func aFileLoadsFromDisk() throws {
        let dir = try Support.scratchDirectory()
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appendingPathComponent("song.gbs")
        try Data(Support.gbs(init: [])).write(to: url)
        var image: OpaquePointer?
        #expect(cgb_image_load_gbs(url.path, &image, nil, 0) == CGB_OK)
        #expect(image != nil)
        cgb_image_free(image)
    }

    @Test func nullArgumentsAreRefused() {
        var image: OpaquePointer?
        #expect(cgb_image_load_gbs(nil, &image, nil, 0) == CGB_ERR_IO)
        #expect(cgb_image_load_gbs("x", nil, nil, 0) == CGB_ERR_IO)
        #expect(cgb_image_from_gbs(nil, 0, &image, nil, 0) == CGB_ERR_FORMAT)
        cgb_image_free(nil)
        #expect(cgb_image_title(nil) == nil)
        #expect(cgb_new(nil, 48000) == nil)
    }

    @Test func anErrorBufferIsCutAndEnded() {
        var image: OpaquePointer?
        var error = [CChar](repeating: 1, count: 8)
        #expect(cgb_image_from_gbs([0], 1, &image, &error, error.count) == CGB_ERR_FORMAT)
        #expect(error[7] == 0)
        #expect(Support.text(error) == "Not a G")
    }
}

extension Support {
    static func text(_ buffer: [CChar]) -> String {
        String(decoding: buffer.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, as: UTF8.self)
    }

    static func scratchDirectory() throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("cgb-tests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }
}
