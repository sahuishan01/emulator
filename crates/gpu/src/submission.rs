use crate::address_space::MAX_BACKING_SPANS;
use crate::{
    decode_pushbuffer, AddressSpaceId, DecodeError, GpFifoEntry, GpuAddressSpace, GpuMemoryError,
    GpuMemoryRange, MethodWrite, MAX_METHOD_WRITES, MAX_PUSHBUFFER_WORDS,
};
use emulator_memory::{Access, BackingRange};
use std::{collections::VecDeque, sync::atomic::AtomicU64};
static NEXT_QUEUE_ID: AtomicU64 = AtomicU64::new(1);
const MAX_PENDING: usize = 16;
const MAX_ENTRIES: usize = 256;
const MAX_BYTES: usize = MAX_PENDING * MAX_PUSHBUFFER_WORDS * 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubmissionId {
    queue: u64,
    serial: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueLimits {
    pub pending: usize,
    pub bytes: usize,
    pub writes: usize,
}
impl Default for QueueLimits {
    fn default() -> Self {
        Self {
            pending: MAX_PENDING,
            bytes: MAX_BYTES,
            writes: MAX_METHOD_WRITES,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueueStats {
    pub pending: usize,
    pub captured_bytes: usize,
    pub decoded_writes: usize,
    pub pinned_spans: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueError {
    InvalidLimits,
    InvalidEntries,
    WrongAddressSpace,
    Capacity,
    IdentityExhausted,
    NotFront,
    Memory(GpuMemoryError),
    Decode(DecodeError),
}
impl std::fmt::Display for QueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GPU submission error: {self:?}")
    }
}
impl std::error::Error for QueueError {}
#[derive(Debug)]
pub struct Submission {
    id: SubmissionId,
    bytes: Vec<u8>,
    writes: Vec<MethodWrite>,
    sources: Vec<GpuMemoryRange>,
}
impl Submission {
    pub fn id(&self) -> SubmissionId {
        self.id
    }
    pub fn captured_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn writes(&self) -> &[MethodWrite] {
        &self.writes
    }
    pub fn source_ranges(&self) -> impl Iterator<Item = BackingRange> + '_ {
        self.sources.iter().flat_map(|s| s.backing_ranges())
    }
}
/// Persistent method state for one synchronous guest channel. Preparation returns
/// an isolated successor and a cancelable effect; neither writes memory.
#[derive(Clone, Debug, Default)]
pub struct ChannelProcessor {
    methods: crate::methods::MethodState,
}
impl ChannelProcessor {
    pub fn prepare(
        &self,
        space: &GpuAddressSpace,
        entries: &[u8],
    ) -> Result<(Self, PreparedSubmission), String> {
        self.prepare_for_class(space, entries, None)
    }
    /// NV contexts may restrict which class a command stream can bind.
    pub fn prepare_for_class(
        &self,
        space: &GpuAddressSpace,
        entries: &[u8],
        class: Option<u32>,
    ) -> Result<(Self, PreparedSubmission), String> {
        let mut queue =
            SubmissionQueue::new(space, QueueLimits::default()).map_err(|e| e.to_string())?;
        queue.methods = self.methods.clone();
        let id = queue.submit(space, entries).map_err(|e| e.to_string())?;
        if let Some(class) = class {
            if queue
                .front()
                .expect("submitted batch")
                .writes()
                .iter()
                .any(|w| w.method_word == 0 && w.data != class)
            {
                return Err("method class does not match allocated context".into());
            }
        }
        let work = queue
            .prepare_front(space, id, crate::ExecutionLimits::default())
            .map_err(|e| e.to_string())?;
        let next = Self {
            methods: work.batch.next_state(),
        };
        Ok((next, work))
    }
}

/// An owned, fully validated batch. Dropping cancels without side effects;
/// commit executes exactly once and returns the queue with its updated state.
#[derive(Debug)]
pub struct PreparedSubmission {
    queue: SubmissionQueue,
    batch: crate::methods::PreparedBatch,
}
impl PreparedSubmission {
    pub fn color_clears(&self) -> &[crate::ColorClear] {
        self.batch.clears()
    }
    pub fn stats(&self) -> crate::ExecutionStats {
        self.batch.stats()
    }
    pub fn write_ranges(&self) -> impl Iterator<Item = BackingRange> + '_ {
        self.batch.write_ranges()
    }
    pub fn commit(mut self) -> (SubmissionQueue, crate::ExecutionStats) {
        let (state, stats) = self.batch.commit();
        self.queue.methods = state;
        self.queue.pending.pop_front();
        (self.queue, stats)
    }
}

#[derive(Debug)]
pub struct SubmissionQueue {
    id: u64,
    owner: AddressSpaceId,
    next: u64,
    limits: QueueLimits,
    pending: VecDeque<Submission>,
    methods: crate::methods::MethodState,
}
impl SubmissionQueue {
    pub fn new(space: &GpuAddressSpace, limits: QueueLimits) -> Result<Self, QueueError> {
        if limits.pending == 0
            || limits.pending > MAX_PENDING
            || limits.bytes == 0
            || limits.bytes > MAX_BYTES
            || limits.writes > MAX_METHOD_WRITES
        {
            return Err(QueueError::InvalidLimits);
        }
        let id = crate::allocate_identity(&NEXT_QUEUE_ID).ok_or(QueueError::IdentityExhausted)?;
        Ok(Self {
            id,
            owner: space.id(),
            next: 1,
            limits,
            pending: VecDeque::new(),
            methods: crate::methods::MethodState::default(),
        })
    }
    pub fn stats(&self) -> QueueStats {
        QueueStats {
            pending: self.pending.len(),
            captured_bytes: self.pending.iter().map(|s| s.bytes.len()).sum(),
            decoded_writes: self.pending.iter().map(|s| s.writes.len()).sum(),
            pinned_spans: self.pending.iter().map(|s| s.source_ranges().count()).sum(),
        }
    }
    pub fn front(&self) -> Option<&Submission> {
        self.pending.front()
    }
    /// Capture immutable command bytes now; retain their original backing until
    /// release. This reference policy is not asynchronous hardware fetch timing.
    pub fn submit(
        &mut self,
        space: &GpuAddressSpace,
        entries: &[u8],
    ) -> Result<SubmissionId, QueueError> {
        if space.id() != self.owner {
            return Err(QueueError::WrongAddressSpace);
        }
        if entries.is_empty() || entries.len() % 8 != 0 || entries.len() / 8 > MAX_ENTRIES {
            return Err(QueueError::InvalidEntries);
        }
        let stats = self.stats();
        if stats.pending >= self.limits.pending {
            return Err(QueueError::Capacity);
        }
        let mut bytes = Vec::new();
        let mut sources = Vec::new();
        let mut spans = stats.pinned_spans;
        for raw in entries.chunks_exact(8) {
            let entry = GpFifoEntry::parse(raw).map_err(QueueError::Decode)?;
            let size = entry.word_count as usize * 4;
            let total = bytes.len() + size;
            if total > MAX_PUSHBUFFER_WORDS * 4
                || total > self.limits.bytes.saturating_sub(stats.captured_bytes)
            {
                return Err(QueueError::Capacity);
            }
            let source = space
                .pin(entry.address, size, Access::Read)
                .map_err(QueueError::Memory)?;
            spans += source.backing_ranges().count();
            if spans > MAX_BACKING_SPANS {
                return Err(QueueError::Capacity);
            }
            let offset = bytes.len();
            bytes.resize(total, 0);
            source
                .read(&mut bytes[offset..])
                .map_err(QueueError::Memory)?;
            sources.push(source);
        }
        // The stream may continue a packet across entries within this batch.
        let writes = decode_pushbuffer(
            &bytes,
            self.limits.writes.saturating_sub(stats.decoded_writes),
        )
        .map_err(QueueError::Decode)?;
        let next = self
            .next
            .checked_add(1)
            .ok_or(QueueError::IdentityExhausted)?;
        let id = SubmissionId {
            queue: self.id,
            serial: self.next,
        };
        self.pending
            .try_reserve(1)
            .map_err(|_| QueueError::Capacity)?;
        self.pending.push_back(Submission {
            id,
            bytes,
            writes,
            sources,
        });
        self.next = next;
        Ok(id)
    }
    /// Consume this queue into a prepared transaction, preserving all pins.
    /// Preparation errors/drop have no memory effects. No mapping changes or
    /// callbacks occur during commit; data is read through its retained pins.
    pub fn prepare_front(
        self,
        space: &GpuAddressSpace,
        id: SubmissionId,
        limits: crate::ExecutionLimits,
    ) -> Result<PreparedSubmission, crate::ExecutionError> {
        if space.id() != self.owner {
            return Err(crate::ExecutionError::WrongAddressSpace);
        }
        let front = self
            .pending
            .front()
            .filter(|s| s.id == id)
            .ok_or(crate::ExecutionError::NotFront)?;
        let batch = self.methods.prepare(space, front.writes(), limits)?;
        Ok(PreparedSubmission { queue: self, batch })
    }
    /// Execute the front batch synchronously, then consume it exactly once.
    /// Validation failure changes neither method state, memory nor the queue.
    /// Data addresses resolve at execution; command bytes were captured at submit.
    pub fn execute_front(
        &mut self,
        space: &GpuAddressSpace,
        id: SubmissionId,
        limits: crate::ExecutionLimits,
    ) -> Result<crate::ExecutionStats, crate::ExecutionError> {
        if space.id() != self.owner {
            return Err(crate::ExecutionError::WrongAddressSpace);
        }
        let front = self
            .pending
            .front()
            .filter(|s| s.id == id)
            .ok_or(crate::ExecutionError::NotFront)?;
        let prepared = self.methods.prepare(space, front.writes(), limits)?;
        let (state, stats) = prepared.commit();
        self.methods = state;
        self.pending.pop_front();
        Ok(stats)
    }
    /// Explicitly release a decoded batch. No GPU methods/fences execute here.
    pub fn release_front(&mut self, id: SubmissionId) -> Result<(), QueueError> {
        if self.pending.front().is_none_or(|s| s.id != id) {
            return Err(QueueError::NotFront);
        }
        self.pending.pop_front();
        Ok(())
    }
}
