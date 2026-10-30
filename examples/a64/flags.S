.text
    movz x0, #0
    subs x0, x0, #1
    svc #1
    adds x0, x0, #1
    svc #2
    movz x1, #0x8000, lsl #48
    subs x2, x1, #1
    svc #3
    adds x2, x2, #1
    svc #5
    movz w1, #0x8000, lsl #16
    subs w2, w1, #1
    svc #4
