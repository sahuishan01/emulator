//! Owned texture/varying semantic and rejection corpus; no GPU or emulator core dependency.
use super::{Case, Counts};
use crate::{decode, decode_program, emit, Error, SampledImage, Stage};
const V: &[u8] = include_bytes!("../../../examples/shaders/texture.vert.bin");
const F: &[u8] = include_bytes!("../../../examples/shaders/texture.frag.bin");
const C: &[u8] = include_bytes!("../../../examples/shaders/varying.frag.bin");
const T: &[u8] = include_bytes!("../../../examples/shaders/texture-transform.vert.bin");
fn require(ok: bool) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err("Texture/varying independent oracle mismatch".into())
    }
}
fn check(i: usize, c: &mut Counts) -> Result<(), String> {
    match i {
        0 => {
            for (code, scale) in [(V, 1.), (T, 0.5)] {
                let p = c.translate(code, Stage::Vertex)?;
                let a = [-1., 1., 0., 1.].map(f32::to_bits);
                let b = [0.25, 0.75, 0., 1.].map(f32::to_bits);
                let out = p.invoke([a, b], None).map_err(|e| e.to_string())?;
                require(
                    out.output == [-scale, scale, 0., 1.].map(f32::to_bits)
                        && out.varying == Some(b)
                        && p.input_mask() == 255,
                )?;
                c.invocations += 1;
                require(p.evaluate(a) == Err(Error::Invocation))?;
                c.rejected += 1;
            }
        }
        1 => {
            let p = c.translate(C, Stage::Fragment)?;
            for input in [[0., 0.25, 0.5, 1.], [-1., 2., 3., 4.]] {
                let input = input.map(f32::to_bits);
                require(p.evaluate(input).map_err(|e| e.to_string())? == input)?;
                c.invocations += 1;
            }
        }
        2 => {
            let p = c.translate(F, Stage::Fragment)?;
            let bytes = [
                0, 64, 128, 255, 255, 128, 64, 0, 23, 47, 89, 137, 255, 255, 255, 255,
            ];
            let image = SampledImage {
                width: 2,
                height: 2,
                pixels: &bytes,
            };
            for (u, v, at) in [
                (-1., -1., 0),
                (0.25, 0.25, 0),
                (0.75, 0.25, 4),
                (0.25, 0.75, 8),
                (1., 1., 12),
                (2., 2., 12),
            ] {
                let q = p
                    .invoke([[u, v, 0., 1.].map(f32::to_bits), [0; 4]], Some(image))
                    .map_err(|e| e.to_string())?;
                require(
                    q.output == std::array::from_fn(|k| (bytes[at + k] as f32 / 255.).to_bits()),
                )?;
                c.invocations += 1;
            }
        }
        3 => {
            let p = c.translate(F, Stage::Fragment)?;
            require(p.evaluate([0; 4]) == Err(Error::TextureBinding))?;
            c.rejected += 1;
            for image in [
                SampledImage {
                    width: 0,
                    height: 1,
                    pixels: &[],
                },
                SampledImage {
                    width: 65,
                    height: 1,
                    pixels: &[],
                },
                SampledImage {
                    width: 1,
                    height: 1,
                    pixels: &[0],
                },
            ] {
                require(p.invoke([[0; 4]; 2], Some(image)).is_err())?;
                c.rejected += 1;
            }
            for x in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                require(
                    p.invoke(
                        [[x, 0., 0., 0.].map(f32::to_bits), [0; 4]],
                        Some(SampledImage {
                            width: 1,
                            height: 1,
                            pixels: &[0; 4],
                        }),
                    )
                    .is_err(),
                )?;
                c.rejected += 1;
            }
            let mut dynamic = F.to_vec();
            dynamic[24..32]
                .copy_from_slice(&(0xe003ff800ff7ff08u64 | (0x88u64 << 28)).to_le_bytes());
            require(decode(&dynamic, Stage::Fragment) == Err(Error::TextureBinding))?;
            c.rejected += 1;
            for (offset, bit) in [(8, 16), (8, 51), (40, 38), (40, 31), (40, 30)] {
                let mut code = F.to_vec();
                let mut w = u64::from_le_bytes(code[offset..offset + 8].try_into().unwrap());
                w ^= 1 << bit;
                code[offset..offset + 8].copy_from_slice(&w.to_le_bytes());
                require(decode(&code, Stage::Fragment).is_err())?;
                c.rejected += 1;
            }
        }
        4 => {
            let page = include_bytes!("../../../examples/gpu/texture-rgba-page.bin");
            for (at, len, stage) in [
                (0xa00, 144, Stage::Vertex),
                (0xe00, 176, Stage::Vertex),
                (0xb00, 144, Stage::Fragment),
            ] {
                let bytes = &page[at..at + len];
                let p = decode_program(bytes, stage).map_err(|e| e.to_string())?;
                c.instructions += p.instruction_count();
                c.spirv_words += emit(&p).map_err(|e| e.to_string())?.len();
                for byte in 0..80 {
                    for bit in 0..8 {
                        let mut mutant = bytes.to_vec();
                        mutant[byte] ^= 1 << bit;
                        require(decode_program(&mutant, stage).is_err())?;
                        c.rejected += 1;
                    }
                }
            }
        }
        5 => {
            for (code, stage) in [
                (V, Stage::Vertex),
                (T, Stage::Vertex),
                (F, Stage::Fragment),
                (C, Stage::Fragment),
            ] {
                for end in 0..code.len() {
                    let _ = decode(&code[..end], stage);
                    c.rejected += usize::from(end % 32 != 0 || end == 0);
                }
                for byte in 0..code.len() {
                    for bit in 0..8 {
                        let mut mutant = code.to_vec();
                        mutant[byte] ^= 1 << bit;
                        if let Ok(p) = decode(&mutant, stage) {
                            let module = emit(&p).map_err(|e| e.to_string())?;
                            require(module.len() <= 2048)?;
                            c.instructions += p.instruction_count();
                            c.spirv_words += module.len();
                            let image = p.texture_handle().map(|_| SampledImage {
                                width: 1,
                                height: 1,
                                pixels: &[1, 2, 3, 4],
                            });
                            let _ = p.invoke([[0; 4]; 2], image);
                            c.invocations += 1;
                        } else {
                            c.rejected += 1;
                        }
                    }
                }
            }
        }
        _ => return Err("Unknown texture shader case".into()),
    }
    Ok(())
}
pub(super) fn run() -> Vec<Case> {
    [
        "vertex-generic-varying-output",
        "fragment-screen-linear-ipa",
        "texture-nearest-rgba-clamp",
        "texture-binding-modes-and-nonfinite-rejected",
        "varying-texture-header-mutations",
        "varying-texture-code-mutations-bounded",
    ]
    .into_iter()
    .enumerate()
    .map(|(i, name)| {
        let mut counts = Counts::default();
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(i, &mut counts)))
                .unwrap_or_else(|_| Err("Texture shader panic".into()));
        Case {
            name,
            passed: result.is_ok(),
            counts,
            error: result.err().unwrap_or_default(),
        }
    })
    .collect()
}
