// Original two-thread NRO fixture, independently assembled by LLVM.
.text
.equ MESSAGE_LENGTH, 49
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
entry:
    adr x20, data
    movz x19, #0x777
    mrs x9, tpidrro_el0
    movz x10, #0x1111
    str x10, [x9, #0x80]
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
    movn w5, #1 // -2: default processor (our single virtual core 0)
    svc #0x08
    cbnz w0, failed
    add x23, x1, #0
    str w23, [x20, #4]
    add x0, x23, #0
    svc #0x09
    cbnz w0, failed
    add x1, x20, #0
    movz w2, #1
    movn x3, #0 // indefinite event wait
    movz w9, #7
    subs wzr, w9, #7 // Z must survive another thread changing its flags
    svc #0x18
    b.ne failed
    cbnz w0, failed
    cbnz w1, failed
    subs xzr, x19, #0x777
    b.ne failed
    ldr x9, [x20, #8]
    subs xzr, x9, #42
    b.ne failed
    mrs x9, tpidrro_el0
    ldr x10, [x9, #0x80]
    sub x10, x10, #0x1000
    subs xzr, x10, #0x111
    b.ne failed
    // Join while the child is sleeping after signaling the event.
    add x1, x20, #4
    movz w2, #1
    movn x3, #0
    svc #0x18
    cbnz w0, failed
    cbnz w1, failed
    add x0, x22, #0
    svc #0x17
    cbnz w0, failed
    // With no producer left, expire this finite wait through virtual time.
    add x1, x20, #0
    movz w2, #1
    movz x3, #100
    svc #0x18
    sub w0, w0, #0xe, lsl #12
    subs wzr, w0, #0xa01
    b.ne failed
    // Timeout must preserve the input handle-array pointer in X1.
    str x1, [x20, #40]
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
    str x0, [x20, #16]
    movz x19, #0x888
    mrs x9, tpidrro_el0
    str x9, [x20, #24]
    ldr x10, [x9, #0x80]
    cbnz x10, failed
    movz x10, #0x2222
    str x10, [x9, #0x80]
    movz x0, #1000
    svc #0x0b
    movz x10, #42
    str x10, [x20, #8]
    movz w9, #3
    subs wzr, w9, #1
    ldr w0, [x20, #48]
    // Writer was supplied through process memory before this point.
    svc #0x11
    cbnz w0, failed
    movz x0, #500
    svc #0x0b
    svc #0x0a
failed:
    .word 0
    .balign 4096
rodata:
message:
    .ascii "Guest threads woke events, joined and timed out.\n"
message_end:
    .if (message_end - message) != MESSAGE_LENGTH
    .error "message length changed"
    .endif
    .balign 4096
data:
    .zero 64
    .balign 4096
image_end:
