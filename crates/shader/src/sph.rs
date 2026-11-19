//! General shader program header (SPH) reader for guest programs.
//!
//! Field positions are written from memory of NVIDIA's public SPH
//! description and are unverified until real programs render correctly.
//! Only what the translator needs is exposed; the output register layout of
//! fragment programs is derived here so the executor, the translator and the
//! report agree on one rule.

/// Header size in bytes; code starts right after it.
pub const BYTES: usize = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Vertex,
    Pixel,
    /// Any other `sph_type` (compute-like or reserved), with the raw value.
    Other(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub words: [u32; 20],
}

/// One colour output component and the register that holds it at EXIT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorOutput {
    pub target: u8,
    pub component: u8,
    pub register: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FragmentOutputs {
    pub colors: Vec<ColorOutput>,
    pub sample_mask: Option<u8>,
    pub depth: Option<u8>,
}

impl Header {
    pub fn parse(bytes: &[u8]) -> Option<Header> {
        let bytes = bytes.get(..BYTES)?;
        let mut words = [0u32; 20];
        for (w, b) in words.iter_mut().zip(bytes.chunks_exact(4)) {
            *w = u32::from_le_bytes(b.try_into().expect("word"));
        }
        Some(Header { words })
    }
    fn bits(&self, word: usize, at: u32, width: u32) -> u32 {
        (self.words[word] >> at) & ((1u32 << width) - 1)
    }
    pub fn kind(&self) -> Kind {
        match self.bits(0, 0, 5) {
            1 => Kind::Vertex,
            2 => Kind::Pixel,
            other => Kind::Other(other as u8),
        }
    }
    pub fn version(&self) -> u32 {
        self.bits(0, 5, 5)
    }
    /// Fragment: writes to more than one render target.
    pub fn mrt_enable(&self) -> bool {
        self.bits(0, 14, 1) != 0
    }
    pub fn kills_pixels(&self) -> bool {
        self.bits(0, 15, 1) != 0
    }
    pub fn sass_version(&self) -> u32 {
        self.bits(0, 17, 4)
    }
    /// Fragment output layout: enabled colour components (4 bits per target
    /// in word 18) take consecutive registers from R0 in target/component
    /// order, then the sample mask, then depth (word 19 bits 0 and 1).
    /// None for non-pixel headers or when the registers run past R254.
    pub fn fragment_outputs(&self) -> Option<FragmentOutputs> {
        if self.kind() != Kind::Pixel {
            return None;
        }
        let mut next = 0u32;
        let mut take = || {
            let r = next;
            next += 1;
            u8::try_from(r).ok().filter(|r| *r < 255)
        };
        let mut colors = Vec::new();
        for target in 0..8u8 {
            for component in 0..4u8 {
                if self.bits(18, u32::from(target) * 4 + u32::from(component), 1) != 0 {
                    colors.push(ColorOutput {
                        target,
                        component,
                        register: take()?,
                    });
                }
            }
        }
        let sample_mask = if self.bits(19, 0, 1) != 0 {
            Some(take()?)
        } else {
            None
        };
        let depth = if self.bits(19, 1, 1) != 0 {
            Some(take()?)
        } else {
            None
        };
        Some(FragmentOutputs {
            colors,
            sample_mask,
            depth,
        })
    }
}
