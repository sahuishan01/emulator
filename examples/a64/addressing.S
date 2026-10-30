.text
    adr x0, target
    adrp x1, target
    movn x2, #0
    movk x2, #0x1234, lsl #32
    movk w2, #0xabcd, lsl #16
    svc #0x42
    .balign 4096
target:
    .quad 0
