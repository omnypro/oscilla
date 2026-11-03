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

    @Test func anNSFeFileGivesItsSongData() throws {
        let extra = [
            Support.chunk("auth", Array("Game\0Artist\0Copy\0Ripper\0".utf8)),
            Support.chunk("text", Array("Note\0".utf8)),
            Support.chunk("tlbl", Array("One\0Two\0".utf8)),
            Support.chunk("taut", Array("\0Second\0".utf8)),
            Support.chunk("time", Support.le32([90_000, 0, -3])),
            Support.chunk("fade", Support.le32([4_000, -1, 7])),
            Support.chunk("plst", [2, 0, 0, 9]),
        ]
        let img = try #require(Support.image(Support.nsfe(init: [], first: 2, chips: 0x04, extra: extra)))
        defer { cnes_image_free(img) }
        #expect(cnes_image_is_nsfe(img) == 1)
        var h = cnes_header()
        cnes_image_header(img, &h)
        #expect(h.songs == 4)
        #expect(h.first_song == 2, "an NSFe file counts from 0")
        #expect(h.chips == 0x04)
        #expect(h.play == 0x8400)
        #expect(String(cString: cnes_image_title(img)) == "Game")
        #expect(String(cString: cnes_image_author(img)) == "Artist")
        #expect(String(cString: cnes_image_copyright(img)) == "Copy")
        #expect(String(cString: cnes_image_ripper(img)) == "Ripper")
        #expect(String(cString: cnes_image_comment(img)) == "Note")
        #expect(String(cString: try #require(cnes_image_song_title(img, 1))) == "Two")
        #expect(String(cString: try #require(cnes_image_song_title(img, 3))) == "")
        #expect(String(cString: try #require(cnes_image_song_author(img, 1))) == "Second")
        #expect(cnes_image_song_title(img, 4) == nil)
        #expect(cnes_image_song_title(img, -1) == nil)
        #expect(cnes_image_song_author(img, 4) == nil)
        #expect(cnes_image_song_time_ms(img, 0) == 90_000)
        #expect(cnes_image_song_fade_ms(img, 0) == 4_000)
        #expect(cnes_image_song_length_ms(img, 0) == 94_000)
        #expect(cnes_image_song_time_ms(img, 1) == 0)
        #expect(cnes_image_song_fade_ms(img, 1) == -1)
        #expect(cnes_image_song_length_ms(img, 1) == 0)
        #expect(cnes_image_song_time_ms(img, 2) == -1)
        #expect(cnes_image_song_length_ms(img, 2) == -1, "a fade without a time gives no length")
        #expect(cnes_image_song_time_ms(img, 3) == -1)
        #expect(cnes_image_song_time_ms(img, 4) == -1)
        #expect(cnes_image_song_length_ms(img, -1) == -1)
        #expect(cnes_image_playlist_count(img) == 3, "the song that does not exist is left out")
        #expect([0, 1, 2].map { cnes_image_playlist_get(img, Int32($0)) } == [2, 0, 0])
        #expect(cnes_image_playlist_get(img, 3) == -1)
        #expect(cnes_image_playlist_get(img, -1) == -1)
    }

    @Test func anNSFFileHasNoSongData() throws {
        let img = try #require(Support.image(Support.nsf(init: [])))
        defer { cnes_image_free(img) }
        #expect(cnes_image_is_nsfe(img) == 0)
        #expect(String(cString: cnes_image_ripper(img)) == "")
        #expect(String(cString: cnes_image_comment(img)) == "")
        #expect(String(cString: try #require(cnes_image_song_title(img, 0))) == "")
        #expect(cnes_image_song_time_ms(img, 0) == -1)
        #expect(cnes_image_song_length_ms(img, 0) == -1)
        #expect(cnes_image_playlist_count(img) == 0)
    }

    @Test func nullImagesGiveNothingForSongData() {
        #expect(cnes_image_is_nsfe(nil) == 0)
        #expect(cnes_image_ripper(nil) == nil)
        #expect(cnes_image_comment(nil) == nil)
        #expect(cnes_image_song_title(nil, 0) == nil)
        #expect(cnes_image_song_author(nil, 0) == nil)
        #expect(cnes_image_song_time_ms(nil, 0) == -1)
        #expect(cnes_image_song_fade_ms(nil, 0) == -1)
        #expect(cnes_image_song_length_ms(nil, 0) == -1)
        #expect(cnes_image_playlist_count(nil) == 0)
        #expect(cnes_image_playlist_get(nil, 0) == -1)
    }

    @Test func anNSFeFilePlaysLikeTheSameNSFFile() throws {
        let start = Support.note
        let play = Support.store(0x4002, 0x80)
        let a = try #require(Support.image(Support.nsf(init: start, play: play, first: 1)))
        let b = try #require(Support.image(Support.nsfe(init: start, play: play)))
        let ma = try #require(cnes_new(a, 48000))
        let mb = try #require(cnes_new(b, 48000))
        cnes_image_free(a)
        cnes_image_free(b)
        defer { cnes_free(ma); cnes_free(mb) }
        let one = Support.render(ma, frames: 9600)
        #expect(one.contains { $0 != 0 })
        #expect(one == Support.render(mb, frames: 9600))
    }

    @Test func aMalformedNSFeFileIsRefused() {
        var image: OpaquePointer?
        var error = [CChar](repeating: 0, count: 128)
        let noInfo = Array("NSFE".utf8) + Support.chunk("DATA", [1])
        #expect(cnes_image_from_nsf(noInfo, noInfo.count, &image, &error, error.count) == CNES_ERR_FORMAT)
        #expect(image == nil)
        #expect(Support.text(error) == "The NSFe file is malformed: there is no INFO chunk.")

        let unknown = Support.nsfe(init: [], extra: [Support.chunk("ZZZZ", [1])])
        #expect(cnes_image_from_nsf(unknown, unknown.count, &image, &error, error.count) == CNES_ERR_FORMAT)
        #expect(Support.text(error) == "The NSFe file needs the chunk ZZZZ, which is not supported.")
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
