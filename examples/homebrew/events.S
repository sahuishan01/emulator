// Original static NRO0 fixture. All labels are resolved by LLVM, not our decoder.
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
    .word 0x1000
    .word 0
    .zero 32
    .zero 32
entry:
    svc #0x45
    cbnz w0, failed
    add x20, x1, #0
    add x21, x2, #0
    adr x19, wait_list
    str w21, [x19]
.macro poll
    add x1, x19, #0
    movz x2, #1
    movz x3, #0
    svc #0x18
.endm
.macro expect value
    sub w0, w0, #(\value >> 12), lsl #12
    subs wzr, w0, #(\value & 0xfff)
    b.ne failed
.endm
    poll
    expect 0xea01
    add x0, x20, #0
    svc #0x11
    cbnz w0, failed
    poll
    cbnz w0, failed
    cbnz w1, failed
    poll
    cbnz w0, failed
    add x0, x21, #0
    svc #0x17
    cbnz w0, failed
    poll
    expect 0xea01
    add x0, x20, #0
    svc #0x11
    cbnz w0, failed
    add x0, x20, #0
    svc #0x12
    cbnz w0, failed
    poll
    expect 0xea01
    add x0, x20, #0
    svc #0x11
    cbnz w0, failed
    add x0, x20, #0
    svc #0x16
    cbnz w0, failed
    poll
    cbnz w0, failed
    add x0, x21, #0
    svc #0x17
    cbnz w0, failed
    add x0, x21, #0
    svc #0x17
    expect 0xfa01
    add x0, x21, #0
    svc #0x16
    cbnz w0, failed
    poll
    expect 0xe401
    movz x22, #7
    adr x10, wait_list
    str x22, [x10, #8]
    adr x0, message
    movz x1, #MESSAGE_LENGTH
    svc #0x27
    svc #0x07
failed:
    .word 0
    .balign 4096
rodata:
message:
    .ascii "Guest events signaled, waited, reset and closed.\n"
message_end:
    .if (message_end - message) != MESSAGE_LENGTH
    .error "message length changed"
    .endif
    .balign 4096
data:
wait_list:
    .zero 16
    .balign 4096
image_end:
