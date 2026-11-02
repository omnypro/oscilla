import Foundation
import Testing
import CGba

/// The GSF container: header, zlib program, tags, and library layering.
struct ImageTests {
    private func load(_ url: URL) -> (cgba_status, OpaquePointer?, String) {
        var image: OpaquePointer?
        var error = [CChar](repeating: 0, count: 256)
        let status = cgba_image_load_gsf(url.path, &image, &error, error.count)
        return (status, image, Support.text(error))
    }

    @Test func aStubLandsOnTopOfItsLibrary() throws {
        let dir = try Support.scratchDirectory()
        defer { try? FileManager.default.removeItem(at: dir) }
        let lib = Support.gsf(load: 0x0800_0000, data: [UInt8](repeating: 0xAA, count: 64),
                              tags: ["title=Library title", "game=The Game", "length=9:59"])
        let stub = Support.gsf(load: 0x0800_0010, data: [1, 2, 3, 4],
                               tags: ["_lib=set.gsflib", "title=The Song", "length=1:15.500", "fade=10"])
        try Data(lib).write(to: dir.appendingPathComponent("set.gsflib"))
        let stubURL = dir.appendingPathComponent("song.minigsf")
        try Data(stub).write(to: stubURL)

        let (status, image, _) = load(stubURL)
        #expect(status == CGBA_OK)
        let img = try #require(image)
        defer { cgba_image_free(img) }
        #expect(cgba_image_rom_size(img) == 64)
        #expect(cgba_image_entry(img) == 0x0800_0000)
        #expect(String(cString: cgba_image_tag(img, "title")) == "The Song")
        #expect(String(cString: cgba_image_tag(img, "game")) == "The Game", "a library fills tags the stub lacks")
        #expect(cgba_image_tag(img, "_lib") == nil)
        #expect(cgba_image_length_ms(img) == 75_500)
        #expect(cgba_image_fade_ms(img) == 10_000)

        let m = try #require(cgba_new(img, 32768))
        defer { cgba_free(m) }
        #expect(cgba_peek(m, 0x0800_000C, 4) == 0xAAAA_AAAA)
        #expect(cgba_peek(m, 0x0800_0010, 4) == 0x0403_0201, "the stub's bytes win")
        #expect(cgba_peek(m, 0x0800_0014, 4) == 0xAAAA_AAAA)
    }

    @Test func missingLibrariesAndBadFilesAreErrors() throws {
        let dir = try Support.scratchDirectory()
        defer { try? FileManager.default.removeItem(at: dir) }
        let stubURL = dir.appendingPathComponent("song.minigsf")
        try Data(Support.gsf(load: 0x0800_0000, data: [0], tags: ["_lib=absent.gsflib"])).write(to: stubURL)
        let (status, image, message) = load(stubURL)
        #expect(status == CGBA_ERR_LIBRARY)
        #expect(image == nil)
        #expect(message.contains("absent.gsflib"))

        let junk = dir.appendingPathComponent("junk.gsf")
        try Data("PSF\u{1}not really".utf8).write(to: junk)
        #expect(load(junk).0 == CGBA_ERR_FORMAT)
        #expect(load(dir.appendingPathComponent("nothing.gsf")).0 == CGBA_ERR_IO)
    }

    @Test func timeTagsReadEveryCommonShape() throws {
        let dir = try Support.scratchDirectory()
        defer { try? FileManager.default.removeItem(at: dir) }
        let cases: [(String, Int32)] = [("75", 75_000), ("75.5", 75_500), ("1:15", 75_000),
                                        ("0:01:15", 75_000), ("2:23.497", 143_497), ("1,5", 1_500)]
        for (text, ms) in cases {
            let url = dir.appendingPathComponent("t.gsf")
            try Data(Support.gsf(load: 0x0800_0000, data: [0], tags: ["length=\(text)"])).write(to: url)
            let (status, image, _) = load(url)
            #expect(status == CGBA_OK)
            #expect(cgba_image_length_ms(image) == ms, "length=\(text)")
            cgba_image_free(image)
        }
    }

    @Test func aMultibootProgramLoadsIntoWorkRam() throws {
        let dir = try Support.scratchDirectory()
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appendingPathComponent("mb.gsf")
        // b . at 0x02000000
        try Data(Support.gsf(entry: 0x0200_0000, load: 0x0200_0000, data: Support.le32(0xEAFF_FFFE))).write(to: url)
        let (status, image, _) = load(url)
        #expect(status == CGBA_OK)
        let m = try #require(cgba_new(image, 32768))
        cgba_image_free(image)
        defer { cgba_free(m) }
        cgba_run_cycles(m, 10_000)
        #expect(cgba_next_pc(m) == 0x0200_0000)
    }
}
