// Original static NRO0 fixture. All labels are resolved by LLVM, not our decoder.
.text
.equ MESSAGE_LENGTH, 37
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
    adr x0, message
    movz x1, #MESSAGE_LENGTH
    svc #0x27
    svc #0x07
    .word 0
    .balign 4096
rodata:
message:
    .ascii "Hello from original Switch homebrew!\n"
message_end:
    .if (message_end - message) != MESSAGE_LENGTH
    .error "message length must match OutputDebugString argument"
    .endif
    .balign 4096
data:
    .quad 0x0123456789abcdef
    .balign 4096
image_end:
