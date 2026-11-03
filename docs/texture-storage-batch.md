# Guest texture storage and compression (0.23)

The guest texture reader now accepts bounded pitch-linear and generic16Bx2
block-linear RGBA8, BC1/DXT1 and BC3/DXT5 images. Encoded guest storage is captured,
untiled and decoded into canonical RGBA8 before the existing shader/raster/Vulkan
path. Native host BC support is unnecessary. The physical Mali report confirms
that dependency matters: native BC formats are unsupported on the target phone.

The home screen's **Test all & export JSON** still does the whole workflow. It now
displays twelve patterns and runs151 diagnostics in one report. This extends the
owned guest graphics profile; it does not add commercial-game loading or general
Horizon graphics services.

## Original formats and layout

Descriptor fields were checked against NVIDIA's
[B197 texture header](https://github.com/NVIDIA/open-gpu-doc/blob/master/classes/3d/clb197tex.h).
The supported component codes are0x08 RGBA8,0x24 DXT1 and0x26 DXT45 with UNORM
components/identity swizzle. The latter is interpreted as non-premultiplied BC3.
Texture type2D, normalized coordinates, nearest/clamp/no-mip sampler and all other
previous interface restrictions remain. sRGB, alternate swizzles/types, layers,
other BC/ASTC formats, implicit LOD and texture-cache optimization remain open.

Header version2 is pitch-linear, with32-byte pitch units, pitch<=256 and32-byte
source alignment. Version3 is block-linear with512-byte source alignment. Only
height exponent bits99:101 may be set in word3; exponents0..5 are accepted.
Width/depth/spacing/3D/mip flags remain zero. Dimensions remain1..64 per axis.

For compression, source rows contain4×4 blocks: BC1 uses8 bytes/block, BC3 uses16.
Partial edge blocks are cropped only after complete blocks are unpacked/decoded.
Pitch padding and padded GOB rows are captured but never sampled as texels.
The storage helper treats RGBA rows and compressed-block rows uniformly.

The byte permutation follows the documented Tegra generic16Bx2 organization,
cross-checked against the primary
[Chromium minigbm Tegra allocator](https://chromium.googlesource.com/chromiumos/platform/minigbm/+/74e4893783bb85d18470b847bd77ba3be57fd632/tegra.c).
A GOB holds64×8 bytes; GOBs stack vertically by the configured power of two.
Row width rounds to64 bytes, height to8×block-height rows. The original forward
address helper and inverse-bit fixture generator are separate implementations.
Fixed byte-offset tests and exhaustive uniqueness checks cover GOB/block boundaries.
This profile does not implement arbitrary PTE kinds, memory compression or3D tiling.

BC rules follow the primary
[Khronos S3TC specification](https://registry.khronos.org/OpenGL/extensions/EXT/EXT_texture_compression_s3tc.txt).
BC1 retains its original three/four-color and transparent-black behavior. BC3
adds both alpha interpolation modes,48-bit3-bit indices and forced four-color RGB
regardless of endpoint order. Reference precision normalizes endpoints, interpolates
and rounds to UNORM8 once, ties up. This is an explicit reference policy, not a
claim of bit-identical interpolation across every native compressed-texture unit.

## Ownership and bounds

The generic codec/storage API checks input sizes, padded footprint, allocation
overflow and a128MiB cap before allocating. The guest reader's much smaller64×64
profile bounds a padded source to64KiB and decoded pixels to16KiB. Converted
pixels are now charged in addition to raw captures/SPIR-V/index storage within
the existing4MiB effect/snapshot budget. An exact-boundary regression demonstrates
that excluding decoded bytes fails admission while including them succeeds.

Pins include the entire padded source. Whole-batch source/output feedback is
rejected by backing identity, including aliases. This conservatively includes
padding. Prepared canonical pixels cannot change after source writes, unmap or
address reuse; later submissions recapture changed compressed alpha. Malformed
descriptors, unreadable/truncated sources and late draw/method failures preserve
the pre-submission bytes and channel state.

## Corpus and bridge

Six authored cases cover tiledRGBA4×4, multi-GOB RGBA17×33, pitched BC1 crop7×5,
tiled BC1 alpha7×9, pitched BC3 alpha5×7 and tiled BC3 alpha13×9. Literal canonical
RGBA fixtures independently check every decoded source byte. Integer sampling
formulas independently check all512 framebuffer bytes per case.

The new `nv-storage.nro` shares only our original NV assembly harness. Its tiled
payload begins at0x400; retained commands move to0x300. It tests the normal guest
CPU/kernel/HIPC/NV route, failed submission, retained owner lifetime and fences.
It exits after5571 instructions/25IPCs/18A+11B descriptors/603buffer bytes/two
copied handles, completion2 and no handles/transfers/loans left. Earlier NRO bytes
remain unchanged. Other format variants use authored command pages through the
same GPU preparation/decoder, then actual Vulkan draws; they are not claims of
six separate interpreted guest executables.

`emulator_texture_storage_replay_v1` is the19th value export and returns the
unchanged99,928-byte TextureReplay. Earlier18 exports/layouts remain stable.
Vulkan receives canonical RGBA images and existing translated guest shaders,
then verifies complete color/alpha readback before window presentation. The
window is opaque; display samples check RGB while native readback checks alpha.

App0.23.0-dev/code26/schema4/workload26/native11 contains151 checks:
engine18/NV50/core30/shader25/Vulkan28, plus12 visible pattern snapshots.
The supplied [0.22.1 physical report](validation/g720-one-tap.md) confirms the
prior134/six-pattern checkpoint; new storage confirmation on Mali is pending.
