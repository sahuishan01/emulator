# Interpolated-depth validation (0.25.0-dev)

The supplied [0.24 physical Mali report](g720-depth.md) confirms all 168 earlier
checks and 21 patterns. The new [interpolation profile](../sloped-depth-batch.md)
is validated below on the host and owned software Android device; its physical
Mali checkpoint remains pending.

All 229 workspace Rust tests, formatting and all-target Clippy with warnings
denied pass. The new integration tests compare every color and D32 value against
pixel-center plane formulas, using the independent general GOB unpacker. They
also reverse triangle winding and restrict scissor. Portable core groups cover
forward/reverse planes, winding, degenerate triangles and disabled writes.
All 31 independent LLVM fixtures are unchanged and verify. Both new command
pages reproduce; the previous nine depth pages are byte-identical.

Fresh host engine 18/NV 50/GPU core 40/shader 25 reports pass. Every preceding
168 named diagnostic record remains identical. Desktop Vulkan passes 39 checks
with validation enabled, zero errors and an empty error field. The new forward
and reverse sloped cases compare every actual GPU color byte and D32 bit with
independent dyadic-plane oracles. No tolerance or expected-image upload is used
to replace the resulting GPU depth values.

Pinned two-ABI Android build and lint pass, as do signature, ELF architecture,
all 20 exports, 16 KiB ELF/ZIP alignment and strict main-app manifest checks.
App/code/schema/workload/native versions are 0.25.0-dev / 28 / 4 / 28 / 13.
The APK is 11,190,556 bytes, SHA-256
`7b260252f34b7f3fcca50ab713d85052d2ef8f6e5cd62f111604ac84d6dc15b5`.
The existing package/signing identity is retained and no permission is added.
Packaging alignment is not a 16 KiB-page Android runtime test.

Automatic Android QA passes all 14 workflow checks, with three fresh completed
172-diagnostic/23-pattern runs. Each report contains only its own ordered pattern
proofs, native pixel/completion/presentation verification and surface updates.
Twenty-three captures independently verify 2,944 displayed pixel centers. Four
fast transitions were missed on the first capture run; the bounded recapture
path collected them in another fully completed run. All completed invocations
created exactly one unique report. No app change or diagnostic relaxation was
needed. Cancellation, abandoned rotation/background attempts, completed rotation,
sharing, cold restoration, upgrade preservation and exact cache/Downloads bytes
pass. The final harness derives summary counts from the actual group records.

At native 1080x1920/density 420, manual QA passes 31 flows, including all 23
patterns, rapid changes, portrait/landscape recreation, background/foreground and
repeated teardown. Twenty-seven captures check 3,456 displayed pixel centers.
The standard export/recovery runner resumes the actual completed report: all
172 cases and four exact host group comparisons, the visible summary, real
picker export, cold restore and both present AtomicFile cache recoveries pass
byte-exact. No new-version Catch snapshot was created, so a third cache recovery
is not claimed. Four UI transitions needed bounded fresh-XML retries.

All three final QA crash buffers are empty, with no target-app fatal exception,
native fatal signal or ANR. Automatic captures temporarily used 720x1280/density
280; native display settings were restored before manual QA. The owned AVD was
started after 96.7% CPU idle, nice 10/two cores/2048 MiB requested (raised to
2560 MiB by the emulator), then shut down and its process awaited. No physical
ADB target was used. The original automatic JSON remains immutable after later
manual scene use.

The sloped-depth automatic screenshot was visually inspected: the left half of
the red rectangle is hidden by the green surface, matching the depth plane.
The reverse-slope manual screenshot was also inspected with the same correct
color division and a separately verified reversed depth gradient.
Raw evidence remains ignored under `reports/android-qa/v0.25.0-dev/` and
`reports/v0.25-*`. General hardware interpolation precision for arbitrary data,
perspective/clipping, depth clears, larger layouts, live guest depth/display and
commercial games are outside this checkpoint.
