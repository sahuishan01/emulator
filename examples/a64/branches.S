.text
    movz x0, #3
loop:
    subs x0, x0, #1
    cbz x0, done
    b loop
done:
    svc #0x42
