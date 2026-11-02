// Original static NRO0 fixture. All labels are resolved by LLVM, not our decoder.
.text
.equ MESSAGE_LENGTH, 69
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
    add x11, x19, #0
    movz w9, #32
.Lzero\@:
    str xzr, [x11]
    add x11, x11, #8
    subs w9, w9, #1
    b.ne .Lzero\@
    adr x10, \source
    add x11, x19, #0
    movz w9, #12
.Lcopy\@:
    ldr x12, [x10]
    str x12, [x11]
    add x10, x10, #8
    add x11, x11, #8
    subs w9, w9, #1
    b.ne .Lcopy\@
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
.macro clear_arguments
    movz x9, #0
    str x9, [x25]
    str x9, [x25, #8]
    str x9, [x25, #16]
    str x9, [x25, #24]
    str x9, [x25, #32]
.endm
.macro install_ioctl source, fd
    install_request \source
    str w25, [x19, #12]
    str w25, [x19, #24]
    str \fd, [x19, #48]
.endm
.macro ioctl_request source, fd
    install_ioctl \source, \fd
    send_nv
.endm
.macro ioctl_input source, fd
    install_request \source
    str w25, [x19, #12]
    str \fd, [x19, #48]
    send_nv
.endm
.macro open_device source, path, fd
    install_request \source
    adr x9, \path
    str w9, [x19, #12]
    send_request x22
    ldr w9, [x19, #36]
    cbnz w9, failed
    ldr \fd, [x19, #32]
    cbz \fd, failed
.endm
.macro close_device fd
    install_request close_request
    str \fd, [x19, #32]
    send_nv
.endm
.macro submit_arguments flags, count, entry, wait
    clear_arguments
    movz w9, #1
    str w9, [x25, #8]
    movz w9, #\flags
    str w9, [x25, #12]
    .if \wait
    str w30, [x25, #16]
    movz w9, #\wait
    str w9, [x25, #20]
    .endif
    movz w9, #\entry
    str w9, [x25, #24]
    movz w9, #(((\count << 10) | 1) & 65535)
    movk w9, #((\count << 10) >> 16), lsl #16
    str w9, [x25, #28]
.endm
.macro expect_error code
    send_request x22
    ldr w9, [x19, #32]
    subs wzr, w9, #\code
    b.ne failed
.endm
.macro check_target value
    ldr x9, [x27, #0x108]
    subs xzr, x9, #\value
    b.ne failed
.endm
.macro check_fence value
    ldr w9, [x25, #16]
    subs wzr, w9, #1
    b.ne failed
    ldr w9, [x25, #20]
    subs wzr, w9, #\value
    b.ne failed
.endm
.macro check_point value
    clear_arguments
    str w30, [x25]
    ioctl_request read_request, w18
    ldr w9, [x25, #4]
    subs wzr, w9, #\value
    b.ne failed
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
    open_device open_nvmap_request, nvmap_path, w23
    adr x25, arguments
    clear_arguments
    movz w9, #20480
    str w9, [x25]
    ioctl_request create_request, w23
    ldr w24, [x25, #4]
    cbz w24, failed
    clear_arguments
    str w24, [x25]
    movz w9, #1
    str w9, [x25, #8]
    adr x27, imported_source
    str x27, [x25, #24]
    ioctl_request alloc_request, w23
    open_device open_as_request, as_path, w28
    clear_arguments
    movz w9, #1
    str w9, [x25]
    ioctl_input as_initialize_request, w28
    clear_arguments
    str w24, [x25, #8]
    ioctl_request map_request, w28
    ldr w9, [x25, #32]
    cbnz w9, failed
    ldr w9, [x25, #36]
    subs wzr, w9, #1
    b.ne failed
    open_device open_gpu_request, gpu_path, w21
    clear_arguments
    str w23, [x25]
    ioctl_input set_nvmap_request, w21
    clear_arguments
    str w21, [x25]
    ioctl_input bind_request, w28
    clear_arguments
    movz w9, #0xb197
    str w9, [x25]
    ioctl_request context_request, w21
    ldr x9, [x25, #8]
    cbz x9, failed
    clear_arguments
    movz w9, #8
    str w9, [x25]
    movz w9, #1
    str w9, [x25, #8]
    ioctl_request fifo_request, w21
    ldr w30, [x25, #12]
    subs wzr, w30, #1
    b.ne failed
    ldr w9, [x25, #16]
    cbnz w9, failed
    open_device open_ctrl_request, ctrl_path, w18

    adr x26, coordinates
    adr x28, mailbox
frame_loop:
    ldr w9, [x28]
    cbnz w9, frame_loop
    ldr w9, [x28, #12]
    cbnz w9, finish
    ldr w9, [x28, #8]
    cbz w9, update
    str wzr, [x28, #8]
    str wzr, [x28, #16]
    str wzr, [x28, #20]
    str wzr, [x28, #24]
    movz w9, #20
    str w9, [x28, #28]
    str wzr, [x28, #32]
update:
    ldr w13, [x28, #4]
    subs wzr, w13, #48
    b.ls paddle_valid
    movz w13, #48
paddle_valid:
    str w13, [x28, #36]
    ldr w10, [x28, #28]
    ldr w11, [x28, #32]
    add w11, w11, #1
    subs wzr, w11, #54
    b.lo positions
    add w12, w10, #4
    subs wzr, w12, w13
    b.ls missed
    add w12, w13, #16
    subs wzr, w10, w12
    b.hs missed
    ldr w9, [x28, #20]
    add w9, w9, #1
    str w9, [x28, #20]
    b respawn
missed:
    ldr w9, [x28, #24]
    add w9, w9, #1
    str w9, [x28, #24]
respawn:
    movz w11, #0
    add w10, w10, #13
    subs wzr, w10, #56
    b.lo positions
    sub w10, w10, #56
positions:
    str w10, [x28, #28]
    str w11, [x28, #32]
    add x14, x27, #0xd00
    add w12, w10, #4
    add w15, w11, #4
    bl rectangle
    ldr w10, [x28, #36]
    movz w11, #58
    add w12, w10, #16
    movz w15, #61
    add x14, x27, #0xd60
    bl rectangle
    submit_arguments 2, 126, 0, 0
    ioctl_request submit_request, w21
    // NV completed only after the synchronous host Vulkan backend returned.
    ldr w9, [x28, #16]
    add w9, w9, #1
    str w9, [x28, #16]
    movz w9, #1
    str w9, [x28]
    b frame_loop
rectangle:
    // x10/x11=left/top, x12/x15=right/bottom. Exact lookup avoids guest FP.
    add x16, x26, x10, lsl #2
    ldr w10, [x16]
    add x16, x26, x11, lsl #2
    ldr w11, [x16]
    add x16, x26, x12, lsl #2
    ldr w12, [x16]
    add x16, x26, x15, lsl #2
    ldr w15, [x16]
    movz w17, #0x3f80, lsl #16
    str w10, [x14]
    str w11, [x14, #4]
    str wzr, [x14, #8]
    str w17, [x14, #12]
    str w12, [x14, #16]
    str w11, [x14, #20]
    str wzr, [x14, #24]
    str w17, [x14, #28]
    str w10, [x14, #32]
    str w15, [x14, #36]
    str wzr, [x14, #40]
    str w17, [x14, #44]
    str w10, [x14, #48]
    str w15, [x14, #52]
    str wzr, [x14, #56]
    str w17, [x14, #60]
    str w12, [x14, #64]
    str w11, [x14, #68]
    str wzr, [x14, #72]
    str w17, [x14, #76]
    str w12, [x14, #80]
    str w15, [x14, #84]
    str wzr, [x14, #88]
    str w17, [x14, #92]
    ret
finish:
    close_device w23
    close_device w21
    close_device w18
    // AS fd retained in argument? Close by known per-client fd2.
    movz w28, #2
    close_device w28
    add x0, x22, #0
    svc #0x16
    cbnz w0, failed
    add x0, x20, #0
    svc #0x16
    cbnz w0, failed
    svc #0x07
failed:
    .word 0
    .balign 8192
rodata:
sm_port:
    .asciz "sm:"
nvmap_path:
    .ascii "/dev/nvmap"
as_path:
    .ascii "/dev/nvhost-as-gpu"
gpu_path:
    .ascii "/dev/nvhost-gpu"
ctrl_path:
    .ascii "/dev/nvhost-ctrl"
message:
    .ascii "Guest draws passed: headers, shaders, pixels, rollback and lifetime.\n"
message_end:
    .if (message_end-message) != MESSAGE_LENGTH
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
    .zero 96 - (. - initialize_request)
service_request:
    .word 4, 10
    .zero 8
    .ascii "SFCI"
    .word 0, 1, 0
    .ascii "nvdrv\0\0\0"
    .zero 96 - (. - service_request)
nv_request:
    .word 4, 0x80000009, 4
    .word 0xffff8001, 0
    .zero 12
    .ascii "SFCI"
    .word 0, 3, 0
    .word 4096
    .zero 96 - (. - nv_request)
open_nvmap_request:
    .word 0x00100004, 8
    .word 10, 0, 0
    .zero 12
    .ascii "SFCI"
    .word 0, 0, 0
    .zero 96 - (. - open_nvmap_request)
open_as_request:
    .word 0x00100004, 8
    .word 18, 0, 0
    .zero 12
    .ascii "SFCI"
    .word 0, 0, 0
    .zero 96 - (. - open_as_request)
open_gpu_request:
    .word 0x00100004, 8
    .word 15, 0, 0
    .zero 12
    .ascii "SFCI"
    .word 0, 0, 0
    .zero 96 - (. - open_gpu_request)
open_ctrl_request:
    .word 0x00100004, 8
    .word 16, 0, 0
    .zero 12
    .ascii "SFCI"
    .word 0, 0, 0
    .zero 96 - (. - open_ctrl_request)
create_request:
    .word 0x1100004, 10
    .word 8, 0, 0
    .word 8, 0, 0
    .ascii "SFCI"
    .word 0, 1, 0
    .word 0, 0xc0080101
    .zero 96 - (. - create_request)
alloc_request:
    .word 0x1100004, 10
    .word 32, 0, 0
    .word 32, 0, 0
    .ascii "SFCI"
    .word 0, 1, 0
    .word 0, 0xc0200104
    .zero 96 - (. - alloc_request)
as_initialize_request:
    .word 0x100004, 10
    .word 40, 0, 0
    .zero 12
    .ascii "SFCI"
    .word 0, 1, 0
    .word 0, 0x40284109
    .zero 96 - (. - as_initialize_request)
map_request:
    .word 0x1100004, 10
    .word 40, 0, 0
    .word 40, 0, 0
    .ascii "SFCI"
    .word 0, 1, 0
    .word 0, 0xc0284106
    .zero 96 - (. - map_request)
set_nvmap_request:
    .word 0x100004, 10
    .word 4, 0, 0
    .zero 12
    .ascii "SFCI"
    .word 0, 1, 0
    .word 0, 0x40044801
    .zero 96 - (. - set_nvmap_request)
bind_request:
    .word 0x100004, 10
    .word 4, 0, 0
    .zero 12
    .ascii "SFCI"
    .word 0, 1, 0
    .word 0, 0x40044101
    .zero 96 - (. - bind_request)
context_request:
    .word 0x1100004, 10
    .word 16, 0, 0
    .word 16, 0, 0
    .ascii "SFCI"
    .word 0, 1, 0
    .word 0, 0xc0104809
    .zero 96 - (. - context_request)
fifo_request:
    .word 0x1100004, 10
    .word 32, 0, 0
    .word 32, 0, 0
    .ascii "SFCI"
    .word 0, 1, 0
    .word 0, 0xc020481a
    .zero 96 - (. - fifo_request)
submit_request:
    .word 0x1100004, 10
    .word 32, 0, 0
    .word 32, 0, 0
    .ascii "SFCI"
    .word 0, 1, 0
    .word 0, 0xc0204808
    .zero 96 - (. - submit_request)
read_request:
    .word 0x1100004, 10
    .word 8, 0, 0
    .word 8, 0, 0
    .ascii "SFCI"
    .word 0, 1, 0
    .word 0, 0xc0080014
    .zero 96 - (. - read_request)
wait_request:
    .word 0x1100004, 10
    .word 16, 0, 0
    .word 16, 0, 0
    .ascii "SFCI"
    .word 0, 1, 0
    .word 0, 0xc0100019
    .zero 96 - (. - wait_request)
close_request:
    .word 4, 9
    .zero 8
    .ascii "SFCI"
    .word 0, 2, 0
    .word 0
    .zero 96 - (. - close_request)
    .balign 4
coordinates:
    .incbin "examples/gpu/demo-coordinates.bin"
    .balign 4096
data:
    .quad 42
    .balign 4096
arguments:
    .zero 64
result_storage:
    .quad 0
    .balign 4096
imported_source:
    .incbin "examples/gpu/demo-page.bin"
    .zero 16384
mailbox:
    .word 0,24,0,0,0,0,0,20,0,24
    .balign 4096
image_end:
