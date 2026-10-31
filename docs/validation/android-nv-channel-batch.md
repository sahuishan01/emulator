# Android combined NV channel batch checkpoint

0.15.0-dev (version code 15), APK SHA-256:
`8696127ee43421a567f6832cff83b8fdb4dbda6267e8b1e31b1664f1a4a421cd`.

Verified 164 Rust tests (including the eighteen-scenario protocol runner), fmt,
Clippy with warnings denied, 24 independent LLVM fixtures (11 raw/13 NROs), pinned
ARM64/x86_64 Rust builds, Gradle build/lint, signature/native identities and 16 KiB
ELF/ZIP packaging alignment. ABI 2 remains 8200 bytes/8192 payload. This is not a
16 KiB Android runtime test. Host report lengths: guest 4924/NV 2354/core 642 bytes.

The Android 15/API 35 x86_64 AVD passes all 40 cases with one Run all checks tap:
13 interpreted NROs (engine suite 10), 18 integrated kernel/NV cases (NV suite 1),
3 GPU-core cases (suite 2), 6 unchanged SwiftShader Vulkan cases. All fields in
all three Rust groups match fresh host reference output except architecture.
Schema 4/workload 15 requires all four groups for combined allPassed. An in-place
upgrade invalidates workload 14 and disables Export until rerun. The screen's
summary is derived from these actual results; missing groups show incomplete checks.

The new guest-nv-channel-batch matches 7179 instructions/32 IPC/2 copies/25 A/
18 B/875 descriptor bytes, expected debug output, completion 3 at image offset 0x4040,
source/target 44 at 0x5100/0x5108, marker 42 at 0x3000 and zero handles/transfer objects/
lent bytes. It covers setup/binding, a partly planned rejected job, state rollback,
valid/repeated jobs, completion reads, completed/future waits, closed allocator/
address-space references and completion after channel destruction. The NV group
independently exercises eighteen success/rejection/rollback/lifetime scenarios
through the actual kernel/service stack; it does not interpret guest CPU bytes.
See [nv-channel-batch.md](../nv-channel-batch.md) for the complete scope and limits.

The complete flow uses tools/qa_android_batch.py. Initial execution passed all 40
native cases and the on-screen summary. The first save navigation outlasted its
initial XML dump; the harness now waits for the expected screen using bounded
fresh dumps and can resume export/restore from a recorded successful batch.
Initial attempt logs/captures remain retained. After an owned-AVD restart the
same report was revalidated, exported byte exactly, and restored byte exactly
after force-stop/cold launch. Summary and enabled Export were verified and both
screenshots inspected. Captured logs show no app crash/ANR and empty crash buffers.
Only emulator-5580 was used; both attempts shut it down explicitly and its process
was awaited. Raw reports, captures, APKs and helper outputs remain ignored.

The supplied [0.14 phone report](g720-nv-gpu-map.md) passes all 21 prior ARM64/G720
checks. This larger batch needs one combined phone report. Results cover owned
synchronous DMA/protocol and native GPU microchecks, not guest rendering,
commercial games, console performance, full async synchronization or broader CPU/
content compatibility. Debug-signed development diagnostic.
