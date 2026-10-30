# Original BC1 RGBA fallback

The CPU reference API is `emulator_texture::decode_bc1_rgba8(input, width,
height)`. It accepts tightly packed row-major BC1 RGBA UNORM blocks, expands
little-endian RGB565 endpoints, applies the four-color or transparent
three-color palette, and crops the last block row/column. Input length must
match exactly. Empty dimensions, overflow and more than 128 MiB decoded data
are rejected before allocation. The limit is per call, not a renderer-wide
memory-budget implementation.

Interpolation uses endpoint precision before one rounding to nearest RGBA8
(ties up). The GPU shader implements the same quantization contract; its
readback must match every byte of the Rust-generated reference image. This is
not a promise of bit-identical native BC sampling across different vendors.
Six independent CPU test cases cover known color vectors, transparency,
equal endpoints, lower RGB565 channel values, image layout, malformed lengths
and resource limits.

```sh
cargo test --offline -p emulator-texture
python3 tools/generate_bc1_fixture.py --check
python3 tools/compile_gpu_shaders.py --check
```

The generated 64x64 corpus tiles eight owned block patterns, mixing four-color
and three-color modes, all selectors, equal/low endpoints and varying index
bits. The GPU receives only ordinary buffer bytes, writes an RGBA8 storage
image and uses explicit compute-to-transfer/host dependencies for readback.
This does not create or sample a native BC texture.

The user's supplied 0.2.0-dev report records this GPU conversion passing on the
physical Mali-G720 driver while native BC support is false. See the
[physical checkpoint](validation/g720-bc1.md). Sampling the resulting RGBA8
image in a draw remains the next test; this corpus checks conversion/readback.

BC1 RGB-only interpretation, sRGB, BC2-BC7, pitched/block-linear guest layouts,
mips/arrays/3D, sampling and guest graphics integration are unsupported. The
original encoded blocks must be retained when future aliases/readbacks need
them. A passing conversion microtest is not a game-rendering result.
