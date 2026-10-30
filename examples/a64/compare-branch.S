.text
    movz x0, #1, lsl #32
    cbz w0, lower_zero
    movz x1, #99
lower_zero:
    cbnz x0, full_nonzero
    movz x1, #98
full_nonzero:
    nop
    movz x1, #42
    svc #0x42
