import Foundation
import CSnes

/// Helpers that the suites share: SPC files built from bytes and machines
/// made from them.
enum Support {
    static let start = 0x0400

    /// An SPC file whose program sits at `start`. Echo writes are off, as after
    /// a reset, so the echo buffer at 0 leaves the zero page alone.
    static func spc(program: [UInt8], dsp: [(Int, UInt8)] = [], ram: [(Int, UInt8)] = [],
                    tagged: Bool = true) -> [UInt8] {
        var file = [UInt8](repeating: 0, count: 0x10200)
        let magic = Array("SNES-SPC700 Sound File Data v0.30".utf8)
        file.replaceSubrange(0..<magic.count, with: magic)
        file[0x21] = 26
        file[0x22] = 26
        file[0x23] = tagged ? 26 : 27
        file[0x24] = 30
        file[0x25] = UInt8(start & 0xFF)
        file[0x26] = UInt8(start >> 8)
        file[0x2B] = 0xEF
        file.replaceSubrange((0x100 + start)..<(0x100 + start + program.count), with: program)
        file[0x10100 + 0x6C] = 0x20
        for (address, value) in ram { file[0x100 + address] = value }
        for (register, value) in dsp { file[0x10100 + register] = value }
        return file
    }

    /// Stores each (register, value) pair in the DSP through ports 0xF2 and
    /// 0xF3 and then spins.
    static func dspProgram(_ writes: [(UInt8, UInt8)]) -> [UInt8] {
        var p: [UInt8] = []
        for (r, v) in writes { p += [0x8F, r, 0xF2, 0x8F, v, 0xF3] }
        return p + [0x2F, 0xFE]
    }

    /// One BRR block with equal nibbles.
    static func block(shift: UInt8, filter: UInt8 = 0, flags: UInt8, nibble: UInt8) -> [UInt8] {
        [shift << 4 | filter << 2 | flags] + [UInt8](repeating: nibble << 4 | nibble, count: 8)
    }

    /// A file that plays a steady voice 0: a looping block of equal nibbles
    /// at full volume.
    static func tone() -> [UInt8] {
        let blockAt = 0x2000
        var ram: [(Int, UInt8)] = [(0x1000, UInt8(blockAt & 0xFF)), (0x1001, UInt8(blockAt >> 8)),
                                   (0x1002, UInt8(blockAt & 0xFF)), (0x1003, UInt8(blockAt >> 8))]
        for (i, b) in block(shift: 12, flags: 3, nibble: 7).enumerated() { ram.append((blockAt + i, b)) }
        let dsp: [(Int, UInt8)] = [
            (0x00, 0x40), (0x01, 0x40), (0x02, 0x00), (0x03, 0x10), (0x04, 0x00),
            (0x05, 0x00), (0x07, 0x7F), (0x0C, 0x7F), (0x1C, 0x7F), (0x5D, 0x10),
            (0x4C, 0x01),
        ]
        return spc(program: [0x2F, 0xFE], dsp: dsp, ram: ram)
    }

    static func image(_ bytes: [UInt8]) -> OpaquePointer {
        var image: OpaquePointer?
        var error = [CChar](repeating: 0, count: 256)
        let status = bytes.withUnsafeBufferPointer {
            csnes_image_from_spc($0.baseAddress, $0.count, &image, &error, error.count)
        }
        precondition(status == CSNES_OK && image != nil, "the file must parse")
        return image!
    }

    static func machine(_ bytes: [UInt8], rate: Int32 = 32000) -> OpaquePointer {
        let img = image(bytes)
        defer { csnes_image_free(img) }
        return csnes_new(img, rate)!
    }

    static func render(_ m: OpaquePointer, frames: Int) -> [Int16] {
        var out = [Int16](repeating: 0, count: frames * 2)
        out.withUnsafeMutableBufferPointer { csnes_render_s16(m, $0.baseAddress, frames) }
        return out
    }
}
