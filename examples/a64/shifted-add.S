// Original integer shifted-register vectors, independently assembled by LLVM.
.text
add x0, x1, x2, lsl #2
adds w0, w1, w2, lsr #31
subs x0, x1, x2, asr #63
add xzr, xzr, xzr
add w0, wzr, w2, lsl #31
sub w0, w1, w2
adds x0, x1, x2
subs w0, w1, w2
add x0, x1, x2, lsr #63
sub w0, w1, w2, asr #31
