//! Diagnostics: which shader programs Z-A binds at refused draws and which
//! instructions they use (see `emulator_shader::census`). Thread-local like
//! the services' nvdrv failure lists; never affects execution results.
use crate::{GpuAddressSpace, GpuVa};
use std::cell::RefCell;
use std::collections::BTreeMap;

const STAGES: [&str; 6] = [
    "VertexA", "VertexB", "TessCtrl", "TessEval", "Geometry", "Fragment",
];
/// Largest program scanned (header excluded).
const MAX_CODE: usize = 64 * 1024;
const MAX_PROGRAMS: usize = 4096;

#[derive(Clone, Debug)]
pub struct ShaderRecord {
    pub stage: &'static str,
    pub draws: u64,
    /// SPH word 0, or None when the header was unreadable.
    pub header: Option<u32>,
    pub census: emulator_shader::census::Census,
    /// Bytes scanned; less than requested when memory ended early.
    pub code_bytes: usize,
    /// Structured decode: instructions decoded, all-zero words, undecodable
    /// words and the first one as (byte offset into code, word).
    pub decoded: usize,
    pub zero_words: usize,
    pub undecoded: usize,
    pub first_undecoded: Option<(usize, u64)>,
    /// Undecodable words with distinct top 16 bits (offset, word).
    pub undecoded_examples: Vec<(usize, u64)>,
    /// Control-flow graph: reachable block count, or why it failed.
    pub cfg: Result<usize, emulator_shader::cfg::CfgError>,
    /// Distinct XMAD and texture instruction forms (offset, decoded), capped.
    pub forms: Vec<(usize, String)>,
    /// Reference executor with every constant and attribute reading zero:
    /// steps-free outcome, or the first explicit failure.
    pub exec: Option<Result<(), emulator_shader::exec::ExecError>>,
    /// SPIR-V translation of pixel programs: module words, or why it failed.
    pub spirv: Option<Result<Vec<u32>, emulator_shader::translate::TranslateError>>,
    /// Full 80-byte header when readable.
    pub sph: Option<emulator_shader::sph::Header>,
}

const MAX_FORMS: usize = 48;

/// Probe environment: every input reads 0.
struct Zeros;
impl emulator_shader::exec::Environment for Zeros {
    fn constant(&self, _: u8, _: u32) -> Option<u32> {
        Some(0)
    }
    fn attribute(&self, _: u16) -> Option<u32> {
        Some(0)
    }
    fn texture_2d(&self, _: u16, _: f32, _: f32, _: f32, _: [i32; 2]) -> Option<[f32; 4]> {
        Some([0.0; 4])
    }
}

fn forms(listing: &emulator_shader::maxwell::Listing, code: &[u8]) -> Vec<(usize, String)> {
    use emulator_shader::maxwell::{Instr, Source};
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for (at, d) in &listing.instructions {
        let word = u64::from_le_bytes(code[*at..*at + 8].try_into().expect("word"));
        // Distinct by flags and operand kind, not register numbers.
        let key = match d.instr {
            Instr::Xmad { b, .. } => {
                let mut i = d.instr;
                if let Instr::Xmad { d, a, c, b: bb, .. } = &mut i {
                    (*d, *a, *c) = (0, 0, 0);
                    if matches!(b, Source::Reg(_)) {
                        *bb = Source::Reg(0);
                    }
                }
                format!("{i:?}")
            }
            Instr::Texs { kind, mask, .. } => format!("Texs kind={kind} mask={mask}"),
            Instr::TexOther { .. } | Instr::Tex { .. } | Instr::Ipa { .. } => {
                format!("top={:#06x}", word >> 48)
            }
            _ => continue,
        };
        if out.len() < MAX_FORMS && seen.insert(key.clone()) {
            out.push((*at, format!("{key} <- {:?} raw={word:#018x}", d.instr)));
        }
    }
    out
}

thread_local! {
    static PROGRAMS: RefCell<BTreeMap<u64, ShaderRecord>> = RefCell::default();
}

/// Record every enabled stage (control word bit 0) at a refused draw.
pub(crate) fn record(space: &GpuAddressSpace, region: u64, pipeline: &[[Option<u32>; 3]; 6]) {
    for (slot, p) in pipeline.iter().enumerate() {
        let (Some(control), Some(offset)) = (p[0], p[1]) else {
            continue;
        };
        if control & 1 == 0 {
            continue;
        }
        let Some(base) = region.checked_add(u64::from(offset)) else {
            continue;
        };
        PROGRAMS.with(|programs| {
            let mut programs = programs.borrow_mut();
            if let Some(record) = programs.get_mut(&base) {
                record.draws += 1;
                return;
            }
            if programs.len() >= MAX_PROGRAMS {
                return;
            }
            let mut header = [0u8; 4];
            let header = space
                .read(GpuVa(base), &mut header)
                .ok()
                .map(|_| u32::from_le_bytes(header));
            let mut sph = [0u8; emulator_shader::sph::BYTES];
            let sph = space
                .read(GpuVa(base), &mut sph)
                .ok()
                .and_then(|_| emulator_shader::sph::Header::parse(&sph));
            let mut code = Vec::new();
            let mut bundle = [0u8; 32];
            while code.len() < MAX_CODE
                && space
                    .read(GpuVa(base + 80 + code.len() as u64), &mut bundle)
                    .is_ok()
            {
                code.extend_from_slice(&bundle);
                if bundle[8..].chunks_exact(8).any(|w| {
                    u64::from_le_bytes(w.try_into().expect("word"))
                        == emulator_shader::census::SELF_BRANCH
                }) {
                    break;
                }
            }
            let listing = emulator_shader::maxwell::list(&code);
            let graph = emulator_shader::cfg::build(&listing);
            let exec = graph.as_ref().ok().map(|g| {
                emulator_shader::exec::run(
                    &listing,
                    g,
                    &Zeros,
                    emulator_shader::exec::State::default(),
                    1 << 20,
                )
                .map(|_| ())
            });
            let spirv = match (&graph, &sph) {
                (Ok(g), Some(h)) if h.kind() == emulator_shader::sph::Kind::Pixel => Some(
                    emulator_shader::translate::translate_fragment(&listing, g, h),
                ),
                _ => None,
            };
            programs.insert(
                base,
                ShaderRecord {
                    stage: STAGES[slot],
                    draws: 1,
                    header,
                    census: emulator_shader::census::scan(&code),
                    code_bytes: code.len(),
                    decoded: listing.instructions.len(),
                    zero_words: listing.zero_words,
                    undecoded: listing.unsupported_count,
                    first_undecoded: listing.unsupported,
                    cfg: graph.map(|g| g.blocks.len()),
                    forms: forms(&listing, &code),
                    exec,
                    spirv,
                    sph,
                    undecoded_examples: listing.undecoded_examples,
                },
            );
        });
    }
}

/// Programs seen at refused draws on this thread, keyed by GPU address.
pub fn shader_census() -> Vec<(u64, ShaderRecord)> {
    PROGRAMS.with(|p| p.borrow().iter().map(|(k, v)| (*k, v.clone())).collect())
}
