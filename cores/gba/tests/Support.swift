import Foundation
import CGba

/// Helpers that the suites share. They build PSF files in memory and run a
/// machine from raw ARM words.
enum Support {
    /// A zlib stream that stores `data` without compressing it: a two-byte
    /// header, one stored deflate block per 65535 bytes, and the Adler-32.
    static func zlibStored(_ data: [UInt8]) -> [UInt8] {
        var out: [UInt8] = [0x78, 0x01]
        var offset = 0
        repeat {
            let n = min(65535, data.count - offset)
            let last: UInt8 = offset + n >= data.count ? 1 : 0
            out.append(last)
            out.append(UInt8(n & 0xFF)); out.append(UInt8(n >> 8))
            out.append(UInt8(~n & 0xFF)); out.append(UInt8((~n >> 8) & 0xFF))
            out.append(contentsOf: data[offset..<offset + n])
            offset += n
        } while offset < data.count
        var a: UInt32 = 1, b: UInt32 = 0
        for byte in data { a = (a + UInt32(byte)) % 65521; b = (b + a) % 65521 }
        let adler = (b << 16) | a
        out.append(contentsOf: [UInt8(adler >> 24), UInt8((adler >> 16) & 0xFF),
                                UInt8((adler >> 8) & 0xFF), UInt8(adler & 0xFF)])
        return out
    }

    /// The text of a NUL-terminated C buffer.
    static func text(_ buffer: [CChar]) -> String {
        String(decoding: buffer.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, as: UTF8.self)
    }

    static func le32(_ v: UInt32) -> [UInt8] {
        [UInt8(v & 0xFF), UInt8((v >> 8) & 0xFF), UInt8((v >> 16) & 0xFF), UInt8(v >> 24)]
    }

    /// A GSF file: the PSF header, the program (entry, load address, size,
    /// data) and optional tag lines.
    static func gsf(entry: UInt32 = 0x0800_0000, load: UInt32, data: [UInt8], tags: [String] = []) -> [UInt8] {
        let program = le32(entry) + le32(load) + le32(UInt32(data.count)) + data
        let packed = zlibStored(program)
        var file: [UInt8] = Array("PSF".utf8) + [0x22]
        file += le32(0) + le32(UInt32(packed.count)) + le32(0)
        file += packed
        if !tags.isEmpty {
            file += Array("[TAG]".utf8)
            file += Array(tags.joined(separator: "\n").utf8)
        }
        return file
    }

    /// A fresh directory under the temporary folder.
    static func scratchDirectory() throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("cgba-tests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    /// A cartridge built from ARM words. It starts at 0x08000000.
    static func rom(words: [UInt32]) -> [UInt8] {
        words.flatMap { le32($0) }
    }

    static func machine(rom bytes: [UInt8], rate: Int32 = 32768) -> OpaquePointer? {
        var image: OpaquePointer?
        guard cgba_image_from_rom(bytes, bytes.count, &image) == CGBA_OK else { return nil }
        defer { cgba_image_free(image) }
        return cgba_new(image, rate)
    }

    /// Runs a test cartridge until it parks in its closing
    /// branch-to-self loop. Returns the register that holds the result, or nil
    /// when the cartridge never parks.
    static func runTestROM(at url: URL, resultRegister: Int32) throws -> UInt32? {
        let bytes = [UInt8](try Data(contentsOf: url))
        guard let m = machine(rom: bytes) else { return nil }
        defer { cgba_free(m) }
        var parked = 0
        for _ in 0..<600 {
            cgba_run_cycles(m, 280_896)
            let pc = cgba_next_pc(m)
            let thumb = cgba_register(m, 16) & 0x20 != 0
            let selfLoop = thumb ? cgba_peek(m, pc, 2) == 0xE7FE : cgba_peek(m, pc, 4) == 0xEAFF_FFFE
            parked = selfLoop ? parked + 1 : 0
            if parked >= 2 { return cgba_register(m, resultRegister) }
        }
        return nil
    }
}
