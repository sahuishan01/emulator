.text
    movz x0, #0xabcd, lsl #48
    movz w0, #0x1234
    movz xzr, #0xffff
    add sp, sp, #16
    add x1, sp, #0
    subs xzr, x1, #16
    svc #0x42
