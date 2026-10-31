# Android transfer-memory/NV initialization checkpoint

0.11.0-dev (version code 11), final APK SHA-256:
`c6bb12d6b2c551e40d816016847509c62f2a326c6c7b2841e60b3ec3c36aae29`.

Verified 137 Rust tests, formatting/Clippy with warnings denied, 20 independent
LLVM fixtures (11 raw A64/nine NROs), pinned ARM64/x86_64 Rust builds, Gradle
build/lint, signature/native identities and 16 KiB ELF/ZIP packaging alignment.
The final build includes the W3-width correction: high X3 bits do not alter the
CreateTransferMemory permission. A host regression test exercises that ABI.
This is not a 16 KiB Android runtime test.

On the isolated Android 15/API 35 x86_64 AVD, workload 11/schema 3 passes all
eighteen checks: nine guest NROs (engine suite 6), three direct GPU-core cases
and six unchanged SwiftShader Vulkan cases. Upgrade from 0.10.0-dev invalidates
the workload 10 report and disables Export. The final APK was installed after
the register-width correction, and that artifact ran the complete suite.

The new guest-nv-transfer-memory-lifetime case matches 317 instructions/four IPC
calls/two copied handles/expected restored-memory output/zero handles. It creates
a 4 KiB transfer object, initializes nvdrv, closes the sender handle, clones the
receiver, closes both receiver handles and reads source value 42 after rights
are restored. FFI also checks zero remaining transfer objects and lent bytes.
The host suite proves restriction persists during receiver retention and that
bad/rejected requests do not adopt the source arena. This is an Initialize RPC
resource subset, not the complete modern libnx bootstrap or an NV device stack.

Document export matches saved JSON byte for byte. A requested force-stop and
cold restart restore identical JSON, the new case and an enabled Export button.
The final screenshot was inspected; taps used fresh validated XML. No startup
dialog intervention was required during the final run. Captured logs show no
app crash/ANR and an empty crash buffer. Exit-info records package update and
the requested force-stop. Only emulator-5580 was used, then shut down. Raw reports,
captures, QA helpers and APKs stay ignored.

The [0.10.0-dev physical report](g720-hipc-buffers.md) passes all seventeen prior
ARM64/G720 cases. This new transfer-memory case needs its own phone report.
NV device descriptors/ioctls, remote transfer maps, shaders/presentation, console
timing/performance and commercial gameplay remain open. This is a debug-signed
development diagnostic.
