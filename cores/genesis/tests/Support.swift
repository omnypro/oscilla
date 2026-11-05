import Foundation
import CGenesis

/// Helpers that the suites share: VGM files built from bytes and machines
/// made from them.
enum Support {
    static let psgClock: UInt32 = 3_579_545
    static let fmClock: UInt32 = 7_670_454

    /// A version 1.71 file whose stream is `stream`. `loopAt` is an offset in
    /// the stream and `loopSamples` the header's length of the loop.
    static func vgm(stream: [UInt8], psg: Bool = true, fm: Bool = true, loopAt: Int? = nil,
                    total: UInt32 = 0, loopSamples: UInt32 = 0, tag: [String]? = nil) -> [UInt8] {
        var file = [UInt8](repeating: 0, count: 0x100)
        func put32(_ at: Int, _ value: UInt32) {
            for i in 0..<4 { file[at + i] = UInt8(truncatingIfNeeded: value >> (8 * UInt32(i))) }
        }
        file.replaceSubrange(0..<4, with: Array("Vgm ".utf8))
        put32(8, 0x171)
        put32(0x18, total)
        put32(0x20, loopSamples)
        put32(0x34, 0x100 - 0x34)
        if psg {
            put32(0x0C, psgClock)
            file[0x28] = 9
            file[0x2A] = 16
        }
        if fm { put32(0x2C, fmClock) }
        if let at = loopAt { put32(0x1C, UInt32(0x100 + at - 0x1C)) }
        file += stream + [0x66]
        if let fields = tag {
            var block: [UInt8] = []
            for f in fields { block += f.utf16.flatMap { [UInt8($0 & 0xFF), UInt8($0 >> 8)] } + [0, 0] }
            put32(0x14, UInt32(file.count - 0x14))
            file += Array("Gd3 ".utf8) + [0, 1, 0, 0]
            file += withUnsafeBytes(of: UInt32(block.count).littleEndian) { Array($0) } + block
        }
        put32(4, UInt32(file.count - 4))
        return file
    }

    static func wait(_ samples: Int) -> [UInt8] {
        [0x61, UInt8(samples & 0xFF), UInt8(samples >> 8)]
    }

    static func fm(_ register: UInt8, _ value: UInt8, part: UInt8 = 0) -> [UInt8] {
        [0x52 + part, register, value]
    }

    /// Channel 1 as one sine carrier at about 440 Hz, keyed on at the end.
    static func fmTone(tl: UInt8 = 0) -> [UInt8] {
        var s: [UInt8] = []
        for (offset, level) in [(0, tl), (4, 127), (8, 127), (12, 127)] as [(UInt8, UInt8)] {
            s += fm(0x30 + offset, 0x01) + fm(0x40 + offset, level) + fm(0x50 + offset, 0x1F)
            s += fm(0x60 + offset, 0) + fm(0x70 + offset, 0) + fm(0x80 + offset, 0x0F)
        }
        s += fm(0xB0, 7) + fm(0xB4, 0xC0) + fm(0xA4, 0x22) + fm(0xA0, 0x4B)
        return s + fm(0x28, 0xF0)
    }

    /// The SN76489's channel 0 as a tone at full volume.
    static func psgTone() -> [UInt8] {
        [0x50, 0x8E, 0x50, 0x0F, 0x50, 0x90]
    }

    static func image(_ bytes: [UInt8]) -> OpaquePointer {
        var image: OpaquePointer?
        var error = [CChar](repeating: 0, count: 256)
        let status = bytes.withUnsafeBufferPointer {
            cgenesis_image_from_vgm($0.baseAddress, $0.count, &image, &error, error.count)
        }
        precondition(status == CGENESIS_OK, "the file did not parse")
        return image!
    }

    static func machine(_ bytes: [UInt8], rate: Int32 = 44100) -> OpaquePointer {
        let img = image(bytes)
        defer { cgenesis_image_free(img) }
        return cgenesis_new(img, rate)!
    }

    static func render(_ machine: OpaquePointer, frames: Int) -> [Float] {
        var out = [Float](repeating: 0, count: frames * 2)
        out.withUnsafeMutableBufferPointer { cgenesis_render_f32(machine, $0.baseAddress, frames) }
        return out
    }

    static func rms(_ samples: [Float]) -> Double {
        let sum = samples.reduce(0.0) { $0 + Double($1) * Double($1) }
        return (sum / Double(max(samples.count, 1))).squareRoot()
    }
}
