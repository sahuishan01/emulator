# First supplied physical G720 report

The user supplied a GPU Lab 0.1.0-dev JSON report after installing the private
development APK. It records a successful run on the target phone. This is
user-supplied device evidence; no physical phone is attached to the development
host through ADB. Raw JSON and attachments remain excluded from Git.

| Field | Reported value |
| --- | --- |
| Phone | realme RMX5061 |
| SoC platform identifier | MT6989 |
| GPU | Mali-G720-Immortalis MC12 |
| Android | 16 / API 36 |
| Vulkan API | 1.3.247, decoded from apiVersionRaw 4206839 |
| Raw driver version | 184553472; no vendor-specific decoding assumed |
| Renderer | Integrated GPU, not reported as software |
| Native BC compression feature | false |
| BC1 / BC3 / BC7 query | Unsupported, VkResult -11 |
| ASTC LDR / ASTC 4x4 query | Advertised / supported for the queried usage |
| Geometry / tessellation | Both advertised; neither stage exercised by this lab |
| Compute | 1024 max workgroup invocations, 32 KiB shared memory |
| Validation layer | Disabled |

All three workloads passed: RGBA8 transfer roundtrip, depth-tested overlapping
triangles and compute-generated vertex visibility. This establishes these
small host Vulkan operations on the reported driver. It does not establish
guest command/shader emulation, sustained performance, texture sampling or
commercial-game compatibility. With validation disabled, the zero validation
error count is not a validation-layer pass. Reported heap capacity is not an
available application-memory budget.

The BC format queries corroborate the missing native path our emulator must
handle. They do not establish that BC handling caused another emulator's Z-A
black screen. Likewise, geometry/tessellation cannot be classified as absent
on this driver merely because the GPU is Mali.

## Resulting implementation

`crates/texture` now implements original reference BC1 RGBA UNORM decoding,
with transparency, precise endpoint interpolation, cropped partial blocks,
checked lengths and bounded allocations. The next GPU Lab adds a fourth test:
an original compute shader converts owned BC1 blocks into an RGBA8 image and
checks every readback byte against the Rust reference fixture. Native BC
support is not required or enabled by that workload.

This is a linear BC1 conversion experiment. BC3/BC7, sRGB transfer behavior,
Tegra block-linear addressing, alias/readback coherence, sampling/presentation
and full guest integration remain separate work. BC1-to-RGBA8 expands texel
storage eightfold for complete blocks, so the future renderer needs a combined
resource budget rather than unrestricted conversion caches.

References: [Vulkan feature definitions](https://docs.vulkan.org/refpages/latest/refpages/source/VkPhysicalDeviceFeatures.html),
[Vulkan result codes](https://docs.vulkan.org/refpages/latest/refpages/source/VkResult.html),
[Microsoft BC1 format description](https://learn.microsoft.com/en-us/windows/win32/direct3d10/d3d10-graphics-programming-guide-resources-block-compression#bc1).
