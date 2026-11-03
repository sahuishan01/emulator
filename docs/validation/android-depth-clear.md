# Expanded depth-clear validation (0.26.0-dev)

The supplied [0.25 physical Mali report](g720-sloped-depth.md) confirms all
172 earlier diagnostics and 23 patterns. This checkpoint adds
[sixteen depth-clear render scenarios and twenty correctness groups](../depth-clear-batch.md).
New physical Mali confirmation remains pending.

All 250 workspace Rust tests, formatting and all-target Clippy with warnings
denied pass. The twenty new portable groups report 257 verified inputs/cycles,
including independent color/depth oracles, invalid state, access failures,
aliases, exact budgets, rollback, retained state and repeated resource release.
All 31 independent LLVM fixtures remain unchanged and verify. All eleven older
depth pages and sixteen new clear pages reproduce from our original generators.

Fresh host engine 18/NV 50/GPU core 60/shader 25 reports pass. Every previous
172 named diagnostic record is unchanged. Host Vulkan passes all 55 checks with
validation enabled, zero errors and an empty error field. Actual Vulkan depth
clears execute against uploaded original data; complete color/D32 readback
matches independently computed expectations. Expected post-clear depth is not
uploaded in place of execution.

Pinned two-ABI Android assembly and lint pass, together with signature, ELF
architecture, all 21 exports, 16 KiB ELF/ZIP alignment and strict main-app
manifest checks. App/code/schema/workload/native versions are
0.26.0-dev / 29 / 4 / 29 / 14. The final APK is 11,650,428 bytes, SHA-256
`97890cbb2f96b3eac81b33d9487206b13d805dd85733967255d516c439d89d5f`.
The package/signing identity is retained and no permission is added. Alignment
verification is not a 16 KiB-page Android runtime test.

Automatic Android QA passes all 14 workflow checks, with three fresh completed
208-diagnostic/39-pattern reports. Every report contains only its own ordered
pattern proofs, native pixel/completion/presentation checks and surface updates.
Thirty-nine captures verify 4,992 displayed pixel centers. Three transitions
were missed during the first capture run; a second complete run collected them.
The initial runner was terminated during the final lifecycle report wait, after
recording capture/share/rotation/background UI evidence. The completed third
report was retained and the final export/cold-restore checks resumed against its
actual bytes. No app result or pixel assertion was relaxed. The harness now
supports this explicit evidence-based `--resume-lifecycle` path. Each completed
run created exactly one unique published app-owned JSON file; abandoned attempts
did not publish. Cancellation, upgrade preservation, sharing and cold restore pass.

The clipped-clear and between-draw clear screenshots were visually inspected;
the green clipped region and newly visible central red rectangle match the
independent oracles. Raw evidence is ignored under
`reports/android-qa/v0.26.0-dev/` and `reports/v0.26-*`.

Manual QA passes 47 flows at native resolution, with 43 captures verifying
5,504 displayed pixel centers. Every pattern, wrap, rapid changes, rotation,
background/foreground and repeated surface/native teardown is covered. The first
39-tap burst finished on variant 38 rather than wrapping to 0 and timed out;
the original trace, UI and logs are retained. Resuming from all completed pattern
captures and repeating the same 39-tap sequence passed, followed by all lifecycle
checks. No app code or input expectation was changed to pass the retry. This
transient input result is recorded rather than attributed conclusively to the
app or automation. The manual single-pixel clear screenshot was also inspected.

Standard export/recovery QA resumes the actual completed automatic report and
passes all 208 cases, four exact Rust-reference comparisons and all 55 native
records. The visible summary, real picker export, cold restoration and both
present AtomicFile cache recoveries are byte-exact. There is no current-version
Catch cache, so a third cache recovery is not claimed. Four UI dumps needed a
bounded fresh-XML retry. The original automatic JSON remains immutable after
manual scene use and Diagnostics reopening.

All three final crash buffers are empty, with no target-app fatal exception,
native fatal signal or ANR. The explicitly owned AVD was shut down and its
process awaited successfully; ADB shows no connected device afterward.

The owned AVD was started after 96.8% CPU idle, nice 10/two cores/
2048 MiB requested (raised to 2560 MiB by the emulator). Automatic captures used
720x1280/density 280, restored to native 1080x1920/density 420 before manual QA.
No physical ADB target was used. Stencil, mixed clear masks, larger layouts,
perspective/clipping, general live guest depth/display and commercial games
remain outside this checkpoint.
