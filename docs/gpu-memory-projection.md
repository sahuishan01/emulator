# Ordered GPU memory projection

This source follow-up to 0.23.0-dev adds an explicit input snapshot and memory
projection to the original synchronous reference executor. It prepares the
memory contract needed by depth and other operations that read preceding writes
before the transaction can publish. It does not enable guest depth commands or
change the released APK's diagnostic profile.

`PreparedSubmission::snapshot_inputs()` consumes and returns the owned plan. It
resolves deferred DMA and masked-clear inputs into the existing scratch buffers,
in command order, without writing guest memory. Each read starts from its pinned
backing and overlays all earlier writes by backing identity and byte offset.
Separate GPU aliases, split/reordered views and repeated backing spans are
handled in the same order as actual publication. A second snapshot preserves the
first one. All input/output pins remain retained until commit or cancellation.

After a snapshot, `read_after(pin, output)` projects this batch's complete writes
onto a readable pin without committing or consuming the plan. The output must
exactly match the pin length and is limited to 4 MiB. Unwritten bytes are read
when this method is called; only prepared writes and their resolved dependencies
are immutable. Write-only pins and invalid lengths fail before modifying output.
Calling it without a snapshot returns `InputsNotSnapshotted`.

Commit of a snapshotted plan publishes its owned buffers exactly once. Dropping
the plan publishes nothing. The default path retains the existing execution-time
DMA and masked-clear reads, including CPU changes between preparation and commit.
No existing caller is implicitly switched to snapshot semantics. The overlay
uses existing buffers and pins, retaining the current 4 MiB effect/capture,
64-launch and 4096-span ceilings; it makes no second copy of all pending writes.
Caller-provided projection output is separate from the owned effect budget.

The live graphics contract also checks that its independently composed raster
oracle matches the pending backing writes. It projects each fully cleared row
through the actual ordered effects before invoking the host renderer. This can
reject a framebuffer whose aliased rows cannot represent the expected image,
without publishing partial bytes or synthesizing read rights for a write-only
target. Its existing full-clear/1..3-draw/no-DMA limits still apply.

Tests compare literal byte results, actual publication and 128 deterministic
mixed transactions against an independent scalar byte oracle and the original
default executor. Cases include masked preservation, partial overlaps, aliases,
split/repeated spans, cancellation, late failures, exact budgets, read rights,
source mutation, idempotence, unmap/address reuse and final backing release.
Existing execution-time behavior tests remain unchanged. A live-frame rejection
case checks aliased physical rows against the raster oracle.

This is a synchronous host reference policy. It does not implement hardware
cache barriers, asynchronous coherence, fences, DMA-to-draw input dependencies,
depth attachment decoding or comparison. The existing draw-input feedback
rejection remains in force. Depth planning must read the projected attachment
at each ordered operation and validate all color/depth writes before publication;
snapshotting a completed plan alone does not implement that behavior.

Verification: all 218 workspace Rust tests pass, including twelve new projection
tests and the aliased-row rejection test. The final GPU tests were rerun after
adding the draw-to-projection comparison. Formatting and workspace/all-target
Clippy with warnings denied pass. All 31 independent LLVM fixtures verify.
Fresh engine/NV/GPU-core/shader records exactly match the 0.23 host baseline.
The rebuilt desktop Vulkan runner passes all 28 checks with validation enabled,
zero validation errors and unchanged per-case records. Logs remain ignored under
`reports/gpu-projection-*`. The physical report validates the released 0.23 APK;
these source changes have not been delivered or tested on the physical phone.
