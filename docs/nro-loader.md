# Initial original NRO execution

The implemented path is `NroImage` -> process mappings -> `Session` -> original
A64 interpreter -> `Kernel::dispatch`. It runs the self-authored
six `examples/homebrew/*.nro` fixtures, assembled and linked from their `.S`
sources by LLVM.
No existing emulator code or console binaries are involved.

Supported format: static, uncompressed NRO0 version 0 with zero flags, a text
segment containing the start/header, and guest-page-aligned file segments.
Text is RX, rodata is R, and data plus zero-initialized BSS is RW. A file can
have trailing homebrew assets, but they are not mounted or interpreted.
MOD0/module relocation, dynamic/embedded metadata, additional/compressed
headers, NSO and full homebrew runtimes are explicitly unsupported.

The parser validates the complete supported header and segment layout before
mapping. A batch mapping is transactional, including conflicts with an existing
address space. File input is limited to 64 MiB, and the image's committed guest
memory to 128 MiB. These are prototype resource limits, not console specifications.

The session currently uses a fixed layout (no ASLR): image at `0x100000`,
read-only entry configuration at `0x7fff0000`, an RW TLS page at `0x7ffe0000`, and a 64 KiB stack at
`0x80000000`. Reserved guard pages surround the stack. The config supplies a
single main-thread handle, application type, two 64-bit SVC availability masks,
and an end marker. X0 points to config, X1 is all ones, LR is zero, and SP is
16-byte aligned. Guest MRS TPIDRRO_EL0 reads `0x7ffe0200`; its first 256 bytes
form the IPC buffer. This does not read any host thread register. The
[reference scheduler](scheduling.md) assigns additional threads their own TLS
pages, contexts and caller-provided stacks.

Implemented calls:

- SVC `0x27`, OutputDebugString: reads X0/X1 as guest address/byte count,
  preserves the bytes in bounded host records and returns W0=0 on success.
- SVC `0x07`, ExitProcess: ends the process without a fabricated guest exit-code
  argument or returning to the following instruction, and releases its handles.
- ConnectToNamedPort, SendSyncRequest and CloseHandle; CreateEvent, SignalEvent,
  ClearEvent, ResetSignal and blocking/polling WaitSynchronization. CreateThread,
  StartThread, ExitThread and SleepThread add a bounded single-core scheduler;
  CancelSynchronization interrupts or retains a pending wait cancellation. Their supported
  behavior and error boundaries are documented in [kernel-ipc.md](kernel-ipc.md).

The debug path allows at most 16 KiB per message, 1 MiB total and 4096 records.
An invalid buffer, exceeded limit or unsupported SVC faults this strict initial
session. Complete Horizon error-result and exception delivery semantics are
not implemented. CLI display escapes non-printing control characters; kernel
records retain the original bytes.

An instruction budget pauses execution, which can resume without repeating
already committed output. An exited/faulted session cannot be executed again.
Guest CPU faults and unsupported calls have separate typed errors.

Memory now supports transactional batches, reserved/committed regions,
cross-region access, protection and unmapping. Entire write ranges are checked
before mutation. Protection/unmapping now retain shared backing slices;
explicit aliases and GPU pins preserve identity/lifetime. Page tables and an
optimized paged allocator remain future work. See [ownership](gpu-memory-submission.md).

Run:

```sh
cargo run --offline -p emulator-cli -- run-nro examples/homebrew/hello.nro
```

Expected output:

```text
Hello from original Switch homebrew!
Guest process exited after 5 instructions.
```

Public interface references: [NRO](https://www.switchbrew.org/wiki/NRO),
[homebrew entry ABI](https://switchbrew.org/wiki/Homebrew_ABI),
[SVC interfaces](https://www.switchbrew.org/wiki/SVC).
