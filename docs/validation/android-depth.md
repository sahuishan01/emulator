# Bounded guest depth validation (0.24.0-dev)

The preceding [physical Mali report](g720-texture-storage.md) confirms the
released 0.23 texture checkpoint. Evidence here covers the new depth batch on
the host and the owned software Android device. Physical 0.24 confirmation is
pending. See the [exact depth profile and limits](../depth-batch.md).

## Engine and native renderer

All 227 workspace Rust tests pass. Eight new GPU integration tests cover ordered
depth effects, immutable inputs, read-only/write-only rights, invalid later draws,
varying/nonfinite/out-of-range Z, exact memory budgets, fragmented span ceilings,
cancellation and backing-alias rejection. The new value-bridge test runs all eight
portable depth groups and nine positive replay fixtures. Final targeted tests
were rerun after source cleanup. Formatting and all-target workspace Clippy with
warnings denied pass. All 31 independently assembled LLVM fixtures are unchanged
and verify; the nine new GPU command-page fixtures reproduce exactly.

Host reports pass engine 18, NV 50, GPU core 38 and shader 25. Every previous
named diagnostic record is unchanged. Rebuilt desktop Vulkan passes all 37
cases with validation enabled, zero validation errors and an empty error field;
the previous 28 native records are identical. New cases read actual D32 depth
and color buffers back and compare every bit/byte with independent oracles.

Pinned two-ABI Android assembly and lint pass. APK signature, arm64/x86_64 ELF,
all 20 exports, 16 KiB ELF/ZIP alignment and strict main-app manifest checks pass.
The debug signing identity and application ID are retained, with no new permission.
App/code/schema/workload/native versions: 0.24.0-dev / 27 / 4 / 27 / 12.
APK: 11,132,888 bytes, SHA-256
`d98b58eaaa295c87d08a87f77f08766f00dfa6627dbe2575377cbc7a3b0299a7`.
Packaging alignment is not a runtime test on an Android device using 16 KiB pages.

## Android workflow and displayed pixels

The automatic workflow passes 14 checks, with two fresh 168-diagnostic/21-pattern
runs. All pattern snapshots belong to their current run and have native pixel,
completion and presentation verification plus a consumer surface update. Twenty-one
captures independently check 2,688 displayed pixel centers. Exact cache/Downloads
bytes, published JSON metadata, one file per completed invocation, cancellation,
sharing, completed rotation and unfinished rotation/background restarts pass.
The actual 0.23-to-0.24 upgrade preserves the library and old report before testing.

An unrelated Messages startup dialog interrupted the first attempt; its Wait
button was selected using fresh UI XML. That attempt remains recorded. The fresh
run completed its rendering/export/lifecycle checks, then exposed a stale final
harness assertion expecting 151 rather than 168. The harness now derives the
count. The final cold-summary check was resumed separately against fresh XML,
verifying the actual 168/21 summary and byte-exact cached/exported JSON. Completed
checks were retained rather than replaced by synthetic results.

At native 1080x1920/density 420, manual scene QA passes 29 checks. All 21 patterns,
wrap, rapid requests, portrait/landscape recreation, background/foreground and
repeated native/surface teardown pass. Twenty-five captures check 3,200 displayed
pixel centers. The near-occlusion automatic image and read-only-depth manual
image were visually inspected. The latter correctly shows red over green because
the earlier near draw did not update stored depth.

The standard export/recovery runner resumes the real completed automatic report.
All four Rust groups match host references apart from architecture. All 168 checks,
visible summary, actual picker export, cold restore and both present AtomicFile
cache recoveries pass byte-exact. The original automatic report remains immutable
after manual scene use. No new-version Catch snapshot was created; a third cache
recovery is not claimed. Four transitions required bounded fresh-XML retries.

All final QA crash buffers are empty and logs contain no target-app fatal
exception, native fatal signal or ANR. Automatic capture temporarily used
720x1280/density 280; native settings were restored before manual QA. The owned
AVD was started after 98% CPU idle, nice 10/two cores/2048 MiB requested (raised
to 2560 MiB by the emulator), then stopped and its process awaited successfully.
No physical ADB device was used. Raw evidence remains ignored under
`reports/android-qa/v0.24.0-dev/` and `reports/v0.24-*`.

This establishes the bounded flat-Z reference/replay path. Depth clears,
interpolation, larger layouts, general live guest display and commercial games
remain open.
