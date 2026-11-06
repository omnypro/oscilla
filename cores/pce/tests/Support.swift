import Foundation
import CPce

/// Helpers that the suites share: HES files built from bytes and machines
/// made from them.
enum Support {
    /// A file whose first ROM bank holds `request` at 0xE100. The vectors
    /// point at an RTI.
    static func hes(request: [UInt8], startSong: UInt8 = 0) -> [UInt8] {
        var bank = [UInt8](repeating: 0, count: 0x2000)
        for (i, b) in request.enumerated() { bank[0x100 + i] = b }
        bank[0x1FF0] = 0x40
        for v in [0x1FF6, 0x1FF8, 0x1FFA, 0x1FFC] {
            bank[v] = 0xF0
            bank[v + 1] = 0xFF
        }
        var f: [UInt8] = Array("HESM".utf8) + [0, startSong, 0x00, 0xE1, 0xFF, 0xF8, 0, 0, 1, 2, 3, 0]
        f += Array("DATA".utf8) + [0x00, 0x20, 0, 0] + [0, 0, 0, 0] + [0, 0, 0, 0]
        return f + bank
    }

    static func sta(_ address: UInt16, _ value: UInt8) -> [UInt8] {
        [0xA9, value, 0x8D, UInt8(address & 0xFF), UInt8(address >> 8)]
    }

    /// Channel 1 at full level in direct D/A mode, with the accumulator (the
    /// song number) written as its word.
    static func songAsWord() -> [UInt8] {
        let psg: UInt16 = 0x0800
        var code: [UInt8] = [0x48]
        code += sta(psg, 0) + sta(psg + 1, 0xFF) + sta(psg + 5, 0xFF) + sta(psg + 4, 0xDF)
        return code + [0x68, 0x8D, 0x06, 0x08, 0x60]
    }

    static func image(_ bytes: [UInt8]) -> OpaquePointer {
        var image: OpaquePointer?
        var error = [CChar](repeating: 0, count: 256)
        let status = bytes.withUnsafeBufferPointer {
            cpce_image_from_hes($0.baseAddress, $0.count, &image, &error, error.count)
        }
        precondition(status == CPCE_OK, "the file did not parse")
        return image!
    }

    static func machine(_ bytes: [UInt8], song: Int32 = 0, rate: Int32 = 44100) -> OpaquePointer {
        let img = image(bytes)
        defer { cpce_image_free(img) }
        return cpce_new(img, rate, song)!
    }

    static func render(_ machine: OpaquePointer, frames: Int) -> [Float] {
        var out = [Float](repeating: 0, count: frames * 2)
        out.withUnsafeMutableBufferPointer { cpce_render_f32(machine, $0.baseAddress, frames) }
        return out
    }
}
