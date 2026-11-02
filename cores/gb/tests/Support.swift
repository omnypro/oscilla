import Foundation
import CGb

/// Helpers that the suites share: GBS files built from bytes and machines
/// made from them.
enum Support {
    static let load = 0x0400
    static let play = 0x0600

    /// `ld a,v; ldh [reg],a`.
    static func store(_ reg: UInt8, _ v: UInt8) -> [UInt8] {
        [0x3E, v, 0xE0, reg]
    }

    /// The sound unit on, both outputs at full volume.
    static var power: [UInt8] {
        store(0x26, 0x80) + store(0x24, 0x77) + store(0x25, 0xFF)
    }

    /// A GBS file whose init and play routines are `init` and `play`. Each
    /// ends with RET.
    static func gbs(init initCode: [UInt8], play: [UInt8] = [], songs: UInt8 = 4,
                    first: UInt8 = 1, tma: UInt8 = 0, tac: UInt8 = 0) -> [UInt8] {
        var data = [UInt8](repeating: 0, count: Self.play - load)
        data.replaceSubrange(0..<initCode.count, with: initCode)
        data[initCode.count] = 0xC9
        data += play
        data.append(0xC9)
        var file = [UInt8](repeating: 0, count: 0x70)
        file.replaceSubrange(0..<4, with: Array("GBS".utf8) + [1])
        file[4] = songs
        file[5] = first
        for (at, v) in [(6, load), (8, load), (10, Self.play), (12, 0xDFFF)] {
            file[at] = UInt8(v & 0xFF)
            file[at + 1] = UInt8(v >> 8)
        }
        file[14] = tma
        file[15] = tac
        file.replaceSubrange(0x10..<0x15, with: Array("Title".utf8))
        file.replaceSubrange(0x30..<0x36, with: Array("Author".utf8))
        file.replaceSubrange(0x50..<0x51, with: Array("?".utf8))
        return file + data
    }

    static func image(_ bytes: [UInt8]) -> OpaquePointer? {
        var image: OpaquePointer?
        var error = [CChar](repeating: 0, count: 128)
        let status = cgb_image_from_gbs(bytes, bytes.count, &image, &error, error.count)
        return status == CGB_OK ? image : nil
    }

    static func machine(init initCode: [UInt8], play: [UInt8] = [], rate: Int32 = 48000) -> OpaquePointer? {
        guard let img = image(gbs(init: initCode, play: play)) else { return nil }
        defer { cgb_image_free(img) }
        return cgb_new(img, rate)
    }

    /// A pulse note on channel 2 at 512 Hz, started in the init routine.
    static var note: [UInt8] {
        power + store(0x16, 0x80) + store(0x17, 0xF0) + store(0x18, 0x00) + store(0x19, 0x87)
    }

    static func render(_ m: OpaquePointer, frames: Int) -> [Int16] {
        var out = [Int16](repeating: 0, count: frames * 2)
        cgb_render_s16(m, &out, frames)
        return out
    }
}
