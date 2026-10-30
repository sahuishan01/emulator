# Converted BC1 sampling and image reuse

GPU Lab 0.3.0-dev adds two workloads to the four-test suite already recorded
in the user's physical G720 report. These new tests require a separate phone
run. They use our host shaders and owned fixtures; guest GPU/shader emulation
and Android swapchain presentation remain unimplemented.

## Workloads

`bc1-converted-texture-sampling` decodes BC1 bytes into an RGBA8 storage image,
transitions that image for fragment sampling, then renders a fullscreen
triangle into another RGBA8 image. A nearest, clamp-to-edge sampler uses
mirrored/scaled coordinates that extend beyond every texture edge. Blending
is disabled so transparent texels' RGBA values can be checked directly.

Every rendered pixel is compared to a CPU texel-coordinate oracle reading the
Rust-generated decoded reference. The oracle uses integer arithmetic at pixel
centers, away from nearest-selection boundaries. It covers all channels,
transparent pixels, orientation and clamped edges, rather than comparing only
a center pixel or comparing two GPU draws with each other.

`bc1-texture-update-resample` supplies changed endpoints/selectors through a
second immutable input buffer and descriptor set, reconverts the same decoded
image, and renders it again. Both dispatch/draw/copy sequences are in **one
command buffer and one submission**, with no host fence between them. The
second readback must match the updated Rust reference, and both encoded input
buffers must retain their original bytes after GPU completion.

The dependencies include host-write to compute-read, compute-write to
fragment-read, fragment-read to the next compute-write, and color-output /
transfer-readback ordering. All resources remain owned by the context through
the submission's completion fence. This is a bounded image reuse test, not
a general conversion-cache, concurrency or eviction implementation.

## Allocation report

`diagnosticAllocations` records the sum of successful Vulkan device-memory
allocation requests and their count across the complete lab run. Resources
are retained until teardown, so those requests also reflect the lab's maximum
concurrent requested backing. They exclude driver-internal allocations,
shader/pipeline storage, Java/native heap usage and process memory. They are
not a measurement of available GPU memory or a gameplay budget.

## Reproduction

```sh
python3 tools/generate_bc1_fixture.py --check
python3 tools/compile_gpu_shaders.py --check
cmake --build target/gpu-lab --parallel 2
EMULATOR_GPU_VALIDATION=1 target/gpu-lab/gpu-lab-cli
```

The suite requires six passing results and workload version 3. Older reports
are not displayed as current results after upgrading the Android app.
The desktop llvmpipe run passes all six checks with the Khronos layer and
synchronization validation requested, with zero validation errors. A temporary
control build dispatches the original source for both images: the first five
checks pass and the update/resample check fails without any Vulkan validation
errors. This verifies that unchanged pixels cannot pass the update oracle.
The control source/binary stays under ignored reports/ and is not shipped.
The Android 15 SwiftShader AVD also passes all six workloads, and upgrading
from 0.2.0-dev invalidates its old cached report. The new physical G720 run is
still pending.
Filtering, sRGB, other BC formats, guest tiling, save/game execution and
sustained physical-device performance remain separate checkpoints.

Vulkan references: [pipeline dependencies](https://docs.vulkan.org/refpages/latest/refpages/source/vkCmdPipelineBarrier.html),
[sampler configuration](https://docs.vulkan.org/refpages/latest/refpages/source/VkSamplerCreateInfo.html).
