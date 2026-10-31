// Original static NRO0 fixture. All labels are resolved by LLVM, not our decoder.
.text
.equ MESSAGE_LENGTH, 54
.equ COMMAND_BYTES, 80
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
.macro install_request source
    adr x10, \source
    .set copy_offset, 0
    .rept 32
    ldr x9, [x10, #copy_offset]
    str x9, [x19, #copy_offset]
    .set copy_offset, copy_offset + 8
    .endr
.endm
.macro send_request handle
    add x0, \handle, #0
    svc #0x21
    cbnz w0, failed
    ldr w9, [x19, #24]
    cbnz w9, failed
.endm
    install_request initialize_request
    send_request x20
    install_request service_request
    send_request x20
    ldr w21, [x19, #12]
    cbz w21, failed
    install_request dma_request
    adr x9, commands
    str x9, [x19, #32]
    movz x9, #1, lsl #32
    add x9, x9, #1, lsl #12
    str x9, [x19, #40]
    movz x9, #COMMAND_BYTES
    str x9, [x19, #48]
    adr x9, data
    str x9, [x19, #56]
    movz x9, #2, lsl #32
    str x9, [x19, #64]
    movz x9, #48
    str x9, [x19, #72]
    adr x9, entries
    str x9, [x19, #80]
    movz x9, #8
    str x9, [x19, #88]
    send_request x21
    ldr x9, [x19, #32]
    subs xzr, x9, #10
    b.ne failed
    ldr x9, [x19, #40]
    subs xzr, x9, #2
    b.ne failed
    ldr x9, [x19, #48]
    subs xzr, x9, #16
    b.ne failed
    ldr x9, [x19, #56]
    subs xzr, x9, #1
    b.ne failed
    adr x10, data
    ldr x9, [x10, #16]
    subs xzr, x9, #42
    b.ne failed
    ldr x9, [x10, #32]
    subs xzr, x9, #42
    b.ne failed
    str x9, [x10, #48]
    install_request count_request
    send_request x21
    ldr x9, [x19, #32]
    subs xzr, x9, #1
    b.ne failed
    add x0, x21, #0
    svc #0x16
    cbnz w0, failed
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
    .ascii "Guest DMA copied 16 bytes; dependent copies verified.\n"
message_end:
    .if (message_end - message) != MESSAGE_LENGTH
    .error "debug message length changed"
    .endif
    .balign 16
initialize_request:
    .word 4, 0x8000000a, 1
    .quad 0
    .zero 12
    .ascii "SFCI"
    .word 0, 0, 0
    .quad 0
    .zero 256 - (. - initialize_request)
service_request:
    .word 4, 10
    .zero 8
    .ascii "SFCI"
    .word 0, 1, 0
    .ascii "emu:gpu\0"
    .zero 256 - (. - service_request)
dma_request:
    .word 4, 24
    .zero 8
    .ascii "SFCI"
    .word 0, 0, 0
    .zero 64
    .zero 256 - (. - dma_request)
count_request:
    .word 4, 8
    .zero 8
    .ascii "SFCI"
    .word 0, 1, 0
    .zero 256 - (. - count_request)
    .balign 4096
data:
    .quad 42
    .zero 40
result_storage:
    .quad 0
    .balign 16
commands:
    // Original literal modern packets: one method/data pair per header.
    .word 0x20010000, 0xb0b5
    .word 0x20010100, 2
    .word 0x20010101, 0
    .word 0x20010102, 2
    .word 0x20010103, 16
    .word 0x20010106, 8
    .word 0x200100c0, 0x182
    .word 0x20010101, 16
    .word 0x20010103, 32
    .word 0x200100c0, 0x182
commands_end:
    .if (commands_end - commands) != COMMAND_BYTES
    .error "command byte length changed"
    .endif
entries:
    .word 0x1000, (((commands_end - commands) / 4) << 10) | 1
    .balign 4096
image_end:
