import Foundation
import Testing
import CPce

@Suite("Images")
struct ImageTests {
    @Test func aFileLoadsAndNamesItsFirstSong() {
        let image = Support.image(Support.hes(request: [0x60], startSong: 7))
        defer { cpce_image_free(image) }
        #expect(cpce_image_start_song(image) == 7)
    }

    @Test func badInputIsRefusedWithASentence() {
        var image: OpaquePointer?
        var error = [CChar](repeating: 0, count: 256)
        let junk: [UInt8] = Array("not an hes file at all".utf8)
        let status = junk.withUnsafeBufferPointer {
            cpce_image_from_hes($0.baseAddress, $0.count, &image, &error, error.count)
        }
        #expect(status == CPCE_ERR_FORMAT)
        #expect(image == nil)
        #expect(String(cString: error) == "Not an HES file.")
        #expect(cpce_image_load_hes("/nonexistent/file.hes", &image, &error, error.count) == CPCE_ERR_IO)
        #expect(cpce_new(nil, 44100, 0) == nil)
    }

    @Test func aRateOutsideTheRangeGivesNoMachine() {
        let image = Support.image(Support.hes(request: [0x60]))
        defer { cpce_image_free(image) }
        #expect(cpce_new(image, 7999, 0) == nil)
        #expect(cpce_new(image, 192_001, 0) == nil)
    }

    @Test func aSongOutsideZeroToTwoFiftyFiveGivesNoMachine() {
        let image = Support.image(Support.hes(request: [0x60]))
        defer { cpce_image_free(image) }
        #expect(cpce_new(image, 44100, 256) == nil)
        #expect(cpce_new(image, 44100, -1) == nil)
        #expect(cpce_new(image, 44100, 255) != nil)
    }
}
