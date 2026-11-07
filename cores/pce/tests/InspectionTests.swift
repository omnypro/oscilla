import Foundation
import Testing
import CPce

@Suite("Inspection")
struct InspectionTests {
    /// Channel 1 with distinct register values, then RTS.
    static func program() -> [UInt8] {
        let psg: UInt16 = 0x0800
        var code = Support.sta(psg, 0) + Support.sta(psg + 1, 0xFE) + Support.sta(psg + 5, 0xAB)
        code += Support.sta(psg + 4, 0x40) + Support.sta(psg + 4, 0x00)
        for i in 0..<32 { code += Support.sta(psg + 6, i < 16 ? 31 : UInt8(i & 7)) }
        code += Support.sta(psg + 2, 0x34) + Support.sta(psg + 3, 0x02) + Support.sta(psg + 4, 0x9F)
        return code + [0x60]
    }

    @Test func theProcessorAndMemoryAreReadable() {
        let m = Support.machine(Support.hes(request: Self.program()))
        defer { cpce_free(m) }
        #expect(cpce_next_pc(m) == 0xE100)
        #expect(cpce_register(m, 4) == cpce_next_pc(m))
        #expect(cpce_peek(m, 0xE100) == 0xA9)
        #expect(cpce_peek(m, 0x10000) == 0)
        _ = Support.render(m, frames: 2000)
        #expect(cpce_next_pc(m) == 0x1FFF)
        #expect(cpce_register(m, 99) == 0)
        #expect(cpce_register(m, -1) == 0)
        #expect(cpce_cycles(m) > 0)
    }

    @Test func theRegisterBlocksHoldWhatTheProgramWrote() {
        let m = Support.machine(Support.hes(request: Self.program()))
        defer { cpce_free(m) }
        _ = Support.render(m, frames: 4000)
        #expect(cpce_register_block_count(m) == 4)
        let sizes = (0..<4).map { cpce_register_block_size(m, $0) }
        #expect(sizes == [40, 192, 8, 4])
        #expect(cpce_register_block_size(m, 4) == 0)
        #expect(cpce_register_block_size(m, -1) == 0)
        #expect(String(cString: cpce_register_block_name(m, 0)) == "PSG")
        #expect(cpce_register_block_name(m, 0) == cpce_register_block_name(m, 0))
        #expect(cpce_register_block_name(m, 4) == nil)
        #expect(cpce_register_block_base(m, 0) == 0)

        var psg = [UInt8](repeating: 0xEE, count: 40)
        #expect(cpce_register_block_get(m, 0, &psg, psg.count) == 40)
        #expect(psg[1] == 0xFE)
        #expect(Array(psg[4..<8]) == [0x34, 0x02, 0x9F, 0xAB])

        var waves = [UInt8](repeating: 0xEE, count: 192)
        #expect(cpce_register_block_get(m, 1, &waves, waves.count) == 192)
        #expect(waves[0] == 31)
        #expect(Array(waves[16..<20]) == [0, 1, 2, 3])

        var one: UInt8 = 0xEE
        #expect(cpce_register_block_get(m, 9, &one, 1) == 0)
        #expect(cpce_register_block_get(m, 0, &one, 0) == 0)
        #expect(cpce_register_block_get(m, 0, nil, 40) == 0)
        #expect(cpce_register_block_get(nil, 0, &one, 1) == 0)
        #expect(one == 0xEE)
    }

    @Test func readingTheMachineDoesNotChangeTheSound() {
        let file = Support.hes(request: Self.program())
        let plain = Support.machine(file)
        let looked = Support.machine(file)
        defer { cpce_free(plain); cpce_free(looked) }
        var buffer = [UInt8](repeating: 0, count: 256)
        for _ in 0..<8 {
            let a = Support.render(plain, frames: 500)
            let b = Support.render(looked, frames: 500)
            #expect(a == b)
            for block in 0..<4 { _ = cpce_register_block_get(looked, Int32(block), &buffer, buffer.count) }
            for i in 0..<14 { _ = cpce_register(looked, Int32(i)) }
            _ = cpce_peek(looked, 0)
            _ = cpce_cycles(looked)
        }
    }

    @Test func theCallsReturnNothingInsideAnObserverCallback() {
        final class Seen { var machine: OpaquePointer?; var values: [UInt32] = [] }
        let seen = Seen()
        let m = Support.machine(Support.hes(request: Self.program()))
        defer { cpce_free(m) }
        seen.machine = m
        let observer = cpce_observer(
            context: Unmanaged.passUnretained(seen).toOpaque(),
            psg_write: { context, _, _, _, _ in
                let s = Unmanaged<Seen>.fromOpaque(context!).takeUnretainedValue()
                s.values.append(cpce_register(s.machine, 4))
                s.values.append(UInt32(cpce_register_block_count(s.machine)))
                s.values.append(UInt32(cpce_register_block_get(s.machine, 0, nil, 0)))
            })
        withExtendedLifetime(seen) {
            var o = observer
            cpce_set_observer(m, &o)
            _ = Support.render(m, frames: 500)
            cpce_set_observer(m, nil)
        }
        #expect(!seen.values.isEmpty)
        #expect(seen.values.allSatisfy { $0 == 0 })
    }
}
