import Foundation
import Testing
import CNes

/// Parsing an NSF file through the C interface.
struct ImageTests {
    @Test func theHeaderComesBack() throws {
        let bytes = Support.nsf(init: [], songs: 7, first: 3, ntscUS: 16666, palUS: 20000,
                                region: 1, chips: 0x05, banks: [0, 1, 2, 3, 4, 5, 6, 7])
        let img = try #require(Support.image(bytes))
        defer { cnes_image_free(img) }
        var h = cnes_header()
        cnes_image_header(img, &h)
        #expect(h.songs == 7)
        #expect(h.first_song == 2, "the file counts from 1 and the interface from 0")
        #expect(h.load == 0x8000)
        #expect(h.`init` == 0x8000)
        #expect(h.play == 0x8400)
        #expect(h.ntsc_us == 16666)
        #expect(h.pal_us == 20000)
        #expect(h.region_bits == 1)
        #expect(h.chips == 0x05)
        withUnsafeBytes(of: h.banks) { #expect(Array($0) == [0, 1, 2, 3, 4, 5, 6, 7]) }
        #expect(String(cString: cnes_image_title(img)) == "Title")
        #expect(String(cString: cnes_image_author(img)) == "Author")
        #expect(String(cString: cnes_image_copyright(img)) == "Copyright 1")
    }

    @Test func aFileMustBeAnNSFFile() {
        var image: OpaquePointer?
        var error = [CChar](repeating: 0, count: 128)
        #expect(cnes_image_from_nsf([1, 2, 3], 3, &image, &error, error.count) == CNES_ERR_FORMAT)
        #expect(image == nil)
        #expect(Support.text(error) == "Not an NSF file.")

        var bad = Support.nsf(init: [])
        bad[5] = 3
        #expect(cnes_image_from_nsf(bad, bad.count, &image, &error, error.count) == CNES_ERR_FORMAT)
        #expect(Support.text(error) == "NSF version 3 is not supported.")

        var none = Support.nsf(init: [])
        none[6] = 0
        #expect(cnes_image_from_nsf(none, none.count, &image, &error, error.count) == CNES_ERR_FORMAT)
        #expect(Support.text(error) == "The file holds no songs.")

        var low = Support.nsf(init: [])
        low[9] = 0x7F
        #expect(cnes_image_from_nsf(low, low.count, &image, &error, error.count) == CNES_ERR_FORMAT)
        #expect(image == nil)
    }

    @Test func aMissingFileIsAnIOError() {
        var image: OpaquePointer?
        var error = [CChar](repeating: 0, count: 128)
        #expect(cnes_image_load_nsf("/nonexistent/file.nsf", &image, &error, error.count) == CNES_ERR_IO)
        #expect(image == nil)
        #expect(!Support.text(error).isEmpty)
    }

    @Test func aFileLoadsFromDisk() throws {
        let dir = try Support.scratchDirectory()
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appendingPathComponent("song.nsf")
        try Data(Support.nsf(init: [])).write(to: url)
        var image: OpaquePointer?
        #expect(cnes_image_load_nsf(url.path, &image, nil, 0) == CNES_OK)
        #expect(image != nil)
        cnes_image_free(image)
    }

    @Test func nullArgumentsAreRefused() {
        var image: OpaquePointer?
        #expect(cnes_image_load_nsf(nil, &image, nil, 0) == CNES_ERR_IO)
        #expect(cnes_image_load_nsf("x", nil, nil, 0) == CNES_ERR_IO)
        #expect(cnes_image_from_nsf(nil, 0, &image, nil, 0) == CNES_ERR_FORMAT)
        cnes_image_free(nil)
        #expect(cnes_image_title(nil) == nil)
        #expect(cnes_image_author(nil) == nil)
        #expect(cnes_image_copyright(nil) == nil)
        #expect(cnes_new(nil, 48000) == nil)
        #expect(cnes_new_for(nil, 48000, CNES_PAL) == nil)
    }

    @Test func anErrorBufferIsCutAndEnded() {
        var image: OpaquePointer?
        var error = [CChar](repeating: 1, count: 8)
        #expect(cnes_image_from_nsf([0], 1, &image, &error, error.count) == CNES_ERR_FORMAT)
        #expect(error[7] == 0)
        #expect(Support.text(error) == "Not an ")
    }
}

extension Support {
    static func text(_ buffer: [CChar]) -> String {
        String(decoding: buffer.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, as: UTF8.self)
    }

    static func scratchDirectory() throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("cnes-tests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }
}
