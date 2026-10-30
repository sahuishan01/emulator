.text
.global _start
_start:
    movz x0, #0
    movz x1, #5
loop:
    add x0, x0, #7
    subs x1, x1, #1
    b.ne loop
    movz x2, #0x2000
    str x0, [x2]
    ldr w3, [x2]
    bl increment
    svc #0x42
increment:
    add x3, x3, #7
    ret
