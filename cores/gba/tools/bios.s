.arm
.text
.org 0x000
    b reset          @ 0x00 reset
    b undef          @ 0x04 undefined instruction
    b undef          @ 0x08 software interrupt (not reached: calls are intercepted)
    b pabort         @ 0x0C prefetch abort
    b dabort         @ 0x10 data abort
    b .              @ 0x14 reserved
    b irq            @ 0x18 interrupt
    b pabort         @ 0x1C fast interrupt
.org 0x0E4
    .word 0xE129F000 @ open-bus value after startup
.org 0x128
irq:
    stmfd sp!, {r0-r3, r12, lr}
    mov r0, #0x04000000
    add lr, pc, #0
    ldr pc, [r0, #-4]
    ldmfd sp!, {r0-r3, r12, lr}
    subs pc, lr, #4
.org 0x144
    .word 0xE55EC002 @ open-bus value after an interrupt
.org 0x17C
swi_exit:
    ldmfd sp!, {r12}
    msr spsr_fc, r12
    ldmfd sp!, {r2, r3, r12, lr}
    movs pc, lr
.org 0x190
    .word 0xE3A02004 @ open-bus value after a BIOS call
.org 0x200
halt:
    stmfd sp!, {r2, r3, r12, lr}
    mrs r12, spsr
    stmfd sp!, {r12}
    mov r12, #0x04000000
    mov r2, #0
    strb r2, [r12, #0x301]
    b swi_exit
.org 0x220
custom_halt:
    stmfd sp!, {r2, r3, r12, lr}
    mrs r12, spsr
    stmfd sp!, {r12}
    mov r12, #0x04000000
    strb r2, [r12, #0x301]
    b swi_exit
.org 0x240
stop:
    stmfd sp!, {r2, r3, r12, lr}
    mrs r12, spsr
    stmfd sp!, {r12}
    mov r12, #0x04000000
    mov r2, #0x80
    strb r2, [r12, #0x301]
    b swi_exit
.org 0x260
vblank_intr_wait:
    mov r0, #1
    mov r1, #1
intr_wait:
    stmfd sp!, {r2, r3, r12, lr}
    mrs r12, spsr
    stmfd sp!, {r12}
    and r12, r12, #0x80
    orr r12, r12, #0x1F
    msr cpsr_c, r12
    mov r12, #0x04000000
    mov r3, #1
    strb r3, [r12, #0x208]
    cmp r0, #0
    ldrhne r3, [r12, #-8]
    bicne r3, r3, r1
    strhne r3, [r12, #-8]
check:
    ldrh r3, [r12, #-8]
    ands r2, r3, r1
    bne done
    mov r2, #0
    strb r2, [r12, #0x301]
    b check
done:
    bic r3, r3, r1
    strh r3, [r12, #-8]
    msr cpsr_c, #0x93
    b swi_exit
reset:
    swi #0
undef:
    movs pc, lr
pabort:
    subs pc, lr, #4
dabort:
    subs pc, lr, #8
stub:
    bx lr
