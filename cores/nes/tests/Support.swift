import Foundation
import CNes

/// Helpers that the suites share: NSF files built from bytes and machines
/// made from them.
enum Support {
    static let load = 0x8000
    static let play = 0x8400

    /// `lda #v; sta addr`.
    static func store(_ address: UInt16, _ v: UInt8) -> [UInt8] {
        [0xA9, v, 0x8D, UInt8(address & 0xFF), UInt8(address >> 8)]
    }

    /// The pulse, triangle and noise channels enabled.
    static var power: [UInt8] {
        store(0x4015, 0x0F)
    }

    /// An NSF file whose init and play routines are `init` and `play`. Each
    /// ends with RTS. The init routine sits at the load address and the play
    /// routine 0x400 bytes after it.
    static func nsf(init initCode: [UInt8], play: [UInt8] = [], songs: UInt8 = 4,
                    first: UInt8 = 1, ntscUS: UInt16 = 0, palUS: UInt16 = 0,
                    region: UInt8 = 0, chips: UInt8 = 0,
                    banks: [UInt8] = [UInt8](repeating: 0, count: 8)) -> [UInt8] {
        var data = [UInt8](repeating: 0, count: Self.play - load)
        data.replaceSubrange(0..<initCode.count, with: initCode)
        data[initCode.count] = 0x60
        data += play
        data.append(0x60)
        var file = [UInt8](repeating: 0, count: 0x80)
        file.replaceSubrange(0..<5, with: Array("NESM".utf8) + [0x1A])
        file[5] = 1
        file[6] = songs
        file[7] = first
        for (at, v) in [(8, load), (0x0A, load), (0x0C, Self.play), (0x6E, Int(ntscUS)), (0x78, Int(palUS))] {
            file[at] = UInt8(v & 0xFF)
            file[at + 1] = UInt8(v >> 8)
        }
        file.replaceSubrange(0x0E..<0x13, with: Array("Title".utf8))
        file.replaceSubrange(0x2E..<0x34, with: Array("Author".utf8))
        file.replaceSubrange(0x4E..<0x59, with: Array("Copyright 1".utf8))
        file.replaceSubrange(0x70..<0x78, with: banks)
        file[0x7A] = region
        file[0x7B] = chips
        return file + data
    }

    static func image(_ bytes: [UInt8]) -> OpaquePointer? {
        var image: OpaquePointer?
        var error = [CChar](repeating: 0, count: 128)
        let status = cnes_image_from_nsf(bytes, bytes.count, &image, &error, error.count)
        return status == CNES_OK ? image : nil
    }

    static func machine(init initCode: [UInt8], play: [UInt8] = [], rate: Int32 = 48000,
                        region: UInt8 = 0, chips: UInt8 = 0) -> OpaquePointer? {
        guard let img = image(nsf(init: initCode, play: play, region: region, chips: chips)) else { return nil }
        defer { cnes_image_free(img) }
        return cnes_new(img, rate)
    }

    /// A machine for the same file on the console that the caller picks.
    static func machine(init initCode: [UInt8], play: [UInt8] = [], rate: Int32 = 48000,
                        region: cnes_region) -> OpaquePointer? {
        guard let img = image(nsf(init: initCode, play: play)) else { return nil }
        defer { cnes_image_free(img) }
        return cnes_new_for(img, rate, region)
    }

    /// A pulse note on channel 1 at about 437 Hz, started in the init routine:
    /// half duty, the length counter halted, full volume.
    static var note: [UInt8] {
        power + store(0x4000, 0xBF) + store(0x4002, 0xFF) + store(0x4003, 0x08)
    }

    static func render(_ m: OpaquePointer, frames: Int) -> [Int16] {
        var out = [Int16](repeating: 0, count: frames * 2)
        cnes_render_s16(m, &out, frames)
        return out
    }
}
