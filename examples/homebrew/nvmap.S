// Original static NRO0 fixture. All labels are resolved by LLVM, not our decoder.
.text
.equ MESSAGE_LENGTH, 60
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
    adr x1, data
    movz x2, #4096
    movz w3, #0
    svc #0x15
    cbnz w0, failed
    add x21, x1, #0
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

.macro send_nv
    send_request x22
    ldr w9, [x19, #32]
    cbnz w9, failed
.endm
.macro ioctl_request source
    install_request \source
    str w25, [x19, #12]
    str w25, [x19, #24]
    str w23, [x19, #48]
    send_nv
.endm
    install_request initialize_request
    send_request x20
    install_request service_request
    send_request x20
    ldr w22, [x19, #12]
    cbz w22, failed
    install_request nv_request
    str w21, [x19, #16]
    send_request x22
    add x0, x21, #0
    svc #0x16
    cbnz w0, failed
    install_request open_request
    adr x9, device_path
    str w9, [x19, #12]
    send_request x22
    ldr w9, [x19, #36]
    cbnz w9, failed
    ldr w23, [x19, #32]
    cbz w23, failed
    // File descriptors and memory ownership survive closing the original session.
    install_request clone_request
    send_request x22
    ldr w26, [x19, #12]
    cbz w26, failed
    add x0, x22, #0
    svc #0x16
    cbnz w0, failed
    add x22, x26, #0
    adr x25, arguments
    movz w9, #4096
    str w9, [x25]
    movz w9, #0
    str w9, [x25, #4]
    ioctl_request create_request
    ldr w24, [x25, #4]
    cbz w24, failed
    str w24, [x25]
    movz w9, #1
    str w9, [x25, #4]
    movz w9, #0
    str w9, [x25, #8]
    ioctl_request param_request
    ldr w9, [x25, #8]
    subs wzr, w9, #1, lsl #12
    b.ne failed
    adr x10, result_storage
    str x9, [x10]
    str w24, [x25]
    movz w9, #0
    str w9, [x25, #4]
    ioctl_request free_request
    ldr x9, [x25, #8]
    cbnz x9, failed
    ldr w9, [x25, #16]
    subs wzr, w9, #1, lsl #12
    b.ne failed
    ldr w9, [x25, #20]
    cbnz w9, failed
    install_request close_request
    str w23, [x19, #32]
    send_nv
    add x0, x22, #0
    svc #0x16
    cbnz w0, failed
    adr x10, data
    ldr x9, [x10]
    subs xzr, x9, #42
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
device_path:
    .ascii "/dev/nvmap"
message:
    .ascii "Guest NVMap created, queried and freed; descriptors closed.\n"
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
    .ascii "nvdrv\0\0\0"
    .zero 256 - (. - service_request)
nv_request:
    .word 4, 0x80000009
    .word 4
    .word 0xffff8001, 0
    .zero 12
    .ascii "SFCI"
    .word 0, 3, 0
    .word 4096
    .zero 256 - (. - nv_request)
clone_request:
    .word 5, 8
    .zero 8
    .ascii "SFCI"
    .word 0, 2, 0
    .zero 256 - (. - clone_request)
open_request:
    .word 0x00100004, 8
    .word 10, 0, 0
    .zero 12
    .ascii "SFCI"
    .word 0, 0, 0
    .zero 256 - (. - open_request)
.macro ioctl_template name, size, code
\name:
    .word 0x01100004, 10
    .word \size, 0, 0
    .word \size, 0, 0
    .ascii "SFCI"
    .word 0, 1, 0
    .word 0, \code
    .zero 256 - (. - \name)
.endm
    ioctl_template create_request, 8, 0xc0080101
    ioctl_template param_request, 12, 0xc00c0109
    ioctl_template free_request, 24, 0xc0180105
close_request:
    .word 4, 9
    .zero 8
    .ascii "SFCI"
    .word 0, 2, 0
    .word 0
    .zero 256 - (. - close_request)
    .balign 4096
data:
    .quad 42
    .balign 4096
arguments:
    .zero 48
result_storage:
    .quad 0
    .balign 4096
image_end:
