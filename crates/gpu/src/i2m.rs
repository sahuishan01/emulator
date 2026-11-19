//! Original inline-to-memory (I2M) engine model, shared by the 3D class and
//! KEPLER_INLINE_TO_MEMORY_B. Register layout follows NVIDIA's public class
//! headers: LINE_LENGTH_IN 0x180, LINE_COUNT 0x184, OFFSET_OUT_UPPER 0x188,
//! OFFSET_OUT 0x18c, PITCH_OUT 0x190, block-linear destination setup
//! 0x194-0x1ac, LAUNCH_DMA 0x1b0, LOAD_INLINE_DATA 0x1b4.
//!
//! Supported: pitch destinations with no completion or flush-only completion.
//! Block-linear destinations, semaphore-release completion, interrupts and
//! unknown launch bits fail explicitly, as does a launch whose data does not
//! arrive completely before another method.
use crate::ExecutionError;

pub(crate) const FIRST: u32 = 0x180;
pub(crate) const LAUNCH: u32 = 0x1b0;
pub(crate) const DATA: u32 = 0x1b4;
/// Bytes one launch may carry.
const MAX_BYTES: u64 = 1 << 20;

#[derive(Clone, Debug, Default)]
pub(crate) struct I2m {
    /// 0x180..=0x1ac, indexed by (offset - 0x180) / 4.
    regs: [Option<u32>; 12],
    pending: Option<Pending>,
}
#[derive(Clone, Debug)]
struct Pending {
    total: usize,
    bytes: Vec<u8>,
}
/// One finished line write: GPU virtual address and bytes.
pub(crate) type LineWrite = (u64, Vec<u8>);

fn error(message: &'static str) -> ExecutionError {
    ExecutionError::InlineToMemory(message)
}

impl I2m {
    pub(crate) fn handles(offset: u32) -> bool {
        (FIRST..=DATA).contains(&offset)
    }
    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some()
    }
    /// Fail if a launch is still waiting for data (called before any other
    /// method and at the end of a batch).
    pub(crate) fn require_idle(&self) -> Result<(), ExecutionError> {
        if self.busy() {
            return Err(error("launch data incomplete"));
        }
        Ok(())
    }
    pub(crate) fn write(
        &mut self,
        offset: u32,
        data: u32,
    ) -> Result<Vec<LineWrite>, ExecutionError> {
        match offset {
            LAUNCH => {
                self.require_idle()?;
                self.launch(data)?;
                Ok(Vec::new())
            }
            DATA => {
                let pending = self.pending.as_mut().ok_or(error("data without launch"))?;
                let take = (pending.total - pending.bytes.len()).min(4);
                pending.bytes.extend_from_slice(&data.to_le_bytes()[..take]);
                if pending.bytes.len() < pending.total {
                    return Ok(Vec::new());
                }
                let bytes = self.pending.take().expect("pending").bytes;
                self.lines(bytes)
            }
            _ => {
                self.require_idle()?;
                self.regs[((offset - FIRST) / 4) as usize] = Some(data);
                Ok(Vec::new())
            }
        }
    }
    fn reg(&self, offset: u32) -> Result<u32, ExecutionError> {
        self.regs[((offset - FIRST) / 4) as usize].ok_or(ExecutionError::UninitializedRegister)
    }
    fn launch(&mut self, flags: u32) -> Result<(), ExecutionError> {
        // bit 0: destination layout (1 = pitch); bits 4-5 completion type
        // (0 none, 1 flush only, 2 release semaphore); bits 8-9 interrupt.
        if flags & 1 == 0 {
            return Err(error("block-linear destination"));
        }
        if (flags >> 4) & 3 > 1 {
            return Err(error("semaphore completion"));
        }
        // bit 6 SYSMEMBAR_DISABLE (no barrier to skip in a coherent model) and
        // bit 12 SEMAPHORE_STRUCT_SIZE (unused without a semaphore) are
        // accepted; reduction (bit 1, 13-17) and other bits fail with the value.
        if flags & !0x1071 != 0 {
            return Err(ExecutionError::InlineToMemoryLaunch(flags));
        }
        let length = u64::from(self.reg(0x180)?);
        let count = u64::from(self.reg(0x184)?);
        let pitch = u64::from(self.reg(0x190)?);
        let total = length * count;
        if total == 0 || total > MAX_BYTES || (count > 1 && pitch < length) {
            return Err(error("invalid line geometry"));
        }
        if self.reg(0x188)? > 0xff {
            return Err(ExecutionError::InvalidAddressUpper(self.reg(0x188)?));
        }
        self.pending = Some(Pending {
            total: total as usize,
            bytes: Vec::with_capacity(total as usize),
        });
        Ok(())
    }
    fn lines(&self, bytes: Vec<u8>) -> Result<Vec<LineWrite>, ExecutionError> {
        let length = self.reg(0x180)? as usize;
        let pitch = u64::from(self.reg(0x190)?);
        let base = u64::from(self.reg(0x188)?) << 32 | u64::from(self.reg(0x18c)?);
        if pitch == length as u64 || bytes.len() == length {
            return Ok(vec![(base, bytes)]);
        }
        Ok(bytes
            .chunks(length)
            .enumerate()
            .map(|(line, chunk)| (base + line as u64 * pitch, chunk.to_vec()))
            .collect())
    }
}
