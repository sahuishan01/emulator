// Original static NRO0 fixture. All labels are resolved by LLVM, not our decoder.
.text
.equ MESSAGE_LENGTH, 46
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
    mrs x19, tpidrro_el0
    adr x1, sm_port
    svc #0x1f
    cbnz w0, failed
    add x20, x1, #0

.macro call_request source, handle
    adr x10, \source
    .set copy_offset, 0
    .rept 32
    ldr x9, [x10, #copy_offset]
    str x9, [x19, #copy_offset]
    .set copy_offset, copy_offset + 8
    .endr
    add x0, \handle, #0
    svc #0x21
    cbnz w0, failed
    ldr w9, [x19, #24]
    cbnz w9, failed
.endm
    call_request initialize_request, x20
    call_request service_request, x20
    ldr w21, [x19, #12]
    cbz w21, failed
    call_request add20_request, x21
    ldr x22, [x19, #32]
    subs xzr, x22, #20
    b.ne failed
    call_request add22_request, x21
    ldr x22, [x19, #32]
    subs xzr, x22, #42
    b.ne failed
    call_request read_request, x21
    ldr x22, [x19, #32]
    subs xzr, x22, #42
    b.ne failed
    adr x10, result_storage
    str x22, [x10]
    add x0, x21, #0
    svc #0x16
    cbnz w0, failed
    add x0, x21, #0
    svc #0x16
    sub w0, w0, #14, lsl #12
    subs wzr, w0, #0x401
    b.ne failed
    add x0, x20, #0
    svc #0x16
    cbnz w0, failed
    adr x0, message
    movz x1, #MESSAGE_LENGTH
    svc #0x27
    svc #0x07
failed:
    .word 0
    .balign 4096
rodata:
sm_port:
    .asciz "sm:"
message:
    .ascii "Guest IPC counter reached 42; handles closed.\n"
message_end:
    .if (message_end - message) != MESSAGE_LENGTH
    .error "debug message length changed"
    .endif
    .balign 16
initialize_request:
    .word 4
    .word 0x8000000a
    .word 1
    .quad 0
    .zero 12
    .ascii "SFCI"
    .word 0
    .word 0
    .word 0
    .quad 0
    .zero 256 - (. - initialize_request)
service_request:
    .word 4
    .word 10
    .zero 8
    .ascii "SFCI"
    .word 0
    .word 1
    .word 0
    .ascii "emu:test"
    .zero 256 - (. - service_request)
add20_request:
    .word 4
    .word 10
    .zero 8
    .ascii "SFCI"
    .word 0
    .word 0
    .word 0
    .quad 20
    .zero 256 - (. - add20_request)
add22_request:
    .word 4
    .word 10
    .zero 8
    .ascii "SFCI"
    .word 0
    .word 0
    .word 0
    .quad 22
    .zero 256 - (. - add22_request)
read_request:
    .word 4
    .word 8
    .zero 8
    .ascii "SFCI"
    .word 0
    .word 1
    .word 0
    .zero 256 - (. - read_request)
    .balign 4096
data:
result_storage:
    .quad 0
    .balign 4096
image_end:
