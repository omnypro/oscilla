import Foundation
import Testing
import CSnes

private final class Log {
    var dsp: [(UInt64, UInt32, UInt32)] = []
    var ports: [(UInt32, UInt32)] = []
    var machine: OpaquePointer?
}

@Suite("Hooks")
struct HooksTests {
    @Test func theObserverSeesEveryDspStoreWithItsClock() {
        let log = Log()
        let m = Support.machine(Support.spc(program: Support.dspProgram([(0x0C, 0x55), (0x1C, 0x66)])))
        defer { csnes_free(m) }
        log.machine = m
        let context = Unmanaged.passUnretained(log).toOpaque()
        var observer = csnes_observer(
            context: context,
            dsp_write: { context, reg, value in
                let log = Unmanaged<Log>.fromOpaque(context!).takeUnretainedValue()
                log.dsp.append((csnes_cycles(log.machine), reg, value))
            },
            port_write: nil)
        csnes_set_observer(m, &observer)
        csnes_run_cycles(m, 200)
        #expect(log.dsp.count == 2)
        #expect(log.dsp.map { $0.1 } == [0x0C, 0x1C])
        #expect(log.dsp.map { $0.2 } == [0x55, 0x66])
        // Each store ends its 5-cycle instruction.
        #expect(log.dsp.map { $0.0 } == [10, 20])
        #expect(csnes_dsp_register(m, 0x0C) == 0x55)
        csnes_set_observer(m, nil)
    }

    @Test func theObserverSeesPortStores() {
        let log = Log()
        // MOV $F6,#42
        let m = Support.machine(Support.spc(program: [0x8F, 0x42, 0xF6, 0x2F, 0xFE]))
        defer { csnes_free(m) }
        let context = Unmanaged.passUnretained(log).toOpaque()
        var observer = csnes_observer(
            context: context,
            dsp_write: nil,
            port_write: { context, port, value in
                Unmanaged<Log>.fromOpaque(context!).takeUnretainedValue().ports.append((port, value))
            })
        csnes_set_observer(m, &observer)
        csnes_run_cycles(m, 100)
        #expect(log.ports.count == 1)
        #expect(log.ports.first?.0 == 2)
        #expect(log.ports.first?.1 == 0x42)
        csnes_set_observer(m, nil)
    }

    @Test func callsFromInsideACallbackDoNothing() {
        let log = Log()
        let m = Support.machine(Support.spc(program: Support.dspProgram([(0x0C, 0x55)])))
        defer { csnes_free(m) }
        log.machine = m
        final class Seen { var restore: Int32 = -1; var peek: UInt32 = 99; var cycles: UInt64 = 0 }
        let seen = Seen()
        let box = Unmanaged.passUnretained(seen).toOpaque()
        struct Pair { var seen: UnsafeMutableRawPointer; var machine: OpaquePointer }
        var pair = Pair(seen: box, machine: m)
        withUnsafeMutablePointer(to: &pair) { p in
            var observer = csnes_observer(
                context: UnsafeMutableRawPointer(p),
                dsp_write: { context, _, _ in
                    let p = context!.assumingMemoryBound(to: Pair.self).pointee
                    let seen = Unmanaged<Seen>.fromOpaque(p.seen).takeUnretainedValue()
                    seen.restore = csnes_snapshot_restore(p.machine, nil)
                    seen.peek = csnes_peek(p.machine, 0x400)
                    seen.cycles = csnes_cycles(p.machine)
                },
                port_write: nil)
            csnes_set_observer(m, &observer)
            csnes_run_cycles(m, 100)
            csnes_set_observer(m, nil)
        }
        #expect(seen.restore == 3)
        #expect(seen.peek == 0)
        #expect(seen.cycles == 10)
    }
}
