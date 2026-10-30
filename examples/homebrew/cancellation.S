// Original guest cancellation fixture; assembled independently with LLVM.
.text
.equ MESSAGE_LENGTH, 51
.global _start
_start:
    b entry
    .word 0
    .quad 0
    .ascii "NRO0"
    .word 0
    .word image_end - _start
    .word 0
    .word 0
    .word rodata - _start
    .word rodata - _start
    .word data - rodata
    .word data - _start
    .word image_end - data
    .word 0x2000
    .word 0
    .zero 32
    .zero 32
.macro expect value
    sub w0, w0, #(\value >> 12), lsl #12
    subs wzr, w0, #(\value & 0xfff)
    b.ne failed
.endm
.macro event_wait timeout
    add x1, x20, #0
    movz w2, #1
    movz x3, #\timeout
    svc #0x18
.endm
entry:
    adr x20, data
    ldr w9, [x0, #8]
    str w9, [x20, #16]
    svc #0x45
    cbnz w0, failed
    add x21, x1, #0
    add x22, x2, #0
    str w21, [x20, #48]
    str w22, [x20]
    adr x1, child
    add x2, x20, #0
    adr x3, image_end
    add x3, x3, #2, lsl #12
    movz w4, #44
    movz w5, #0
    svc #0x08
    cbnz w0, failed
    add x23, x1, #0
    str w23, [x20, #4]
    // Two requests before StartThread coalesce into one deferred interruption.
    add x0, x23, #0
    svc #0x19
    cbnz w0, failed
    add x0, x23, #0
    svc #0x19
    cbnz w0, failed
    add x0, x23, #0
    svc #0x09
    cbnz w0, failed
    add x1, x20, #0
    movz w2, #1
    movn x3, #0
    svc #0x18
    str x1, [x20, #24]
    expect 0xec01
    // Cancellation did not signal the object or leave another pending request.
    event_wait 0
    expect 0xea01
    add x1, x20, #4
    movz w2, #1
    movn x3, #0
    svc #0x18
    cbnz w0, failed
    cbnz w1, failed
    event_wait 0
    cbnz w0, failed
    cbnz w1, failed
    add x0, x22, #0
    svc #0x12 // ClearEvent accepts readable as well as writable handles.
    cbnz w0, failed
    event_wait 0
    expect 0xea01
    add x0, x23, #0
    svc #0x16
    cbnz w0, failed
    add x0, x21, #0
    svc #0x16
    cbnz w0, failed
    add x0, x22, #0
    svc #0x16
    cbnz w0, failed
    movz x9, #42
    str x9, [x20, #32]
    adr x0, message
    movz x1, #MESSAGE_LENGTH
    svc #0x27
    svc #0x07
child:
    add x20, x0, #0
    event_wait 0
    expect 0xea01
    event_wait 50
    str x1, [x20, #40]
    expect 0xec01
    ldr w0, [x20, #16]
    svc #0x19 // Interrupt the main thread's already-blocked event wait.
    cbnz w0, failed
    movz x0, #1000
    svc #0x0b
    ldr w0, [x20, #48]
    // Writer was placed in process memory before StartThread.
    svc #0x11
    cbnz w0, failed
    svc #0x0a
failed:
    .word 0
    .balign 4096
rodata:
message:
    .ascii "Guest cancellation interrupted waits exactly once.\n"
message_end:
    .if (message_end - message) != MESSAGE_LENGTH
    .error "message length changed"
    .endif
    .balign 4096
data:
    .zero 64
    .balign 4096
image_end:
