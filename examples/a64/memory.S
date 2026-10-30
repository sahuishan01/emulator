.text
    movz x0, #0x2000
    movz x1, #0x1234, lsl #48
    str x1, [x0, #8]
    ldr x2, [x0, #8]
    movz w3, #0xbeef, lsl #16
    str w3, [x0, #4]
    ldr w4, [x0, #4]
    svc #0x42
