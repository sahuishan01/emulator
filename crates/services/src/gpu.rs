//! Emulator-owned one-shot DMA bridge. This is not the nvdrv wire protocol.
use emulator_gpu::{
    ExecutionLimits, GpuAddressSpace, GpuVa, PreparedSubmission, QueueLimits, SubmissionQueue,
};
use emulator_ipc::Request;
use emulator_kernel::service::{
    PreparedServiceEffect, Service, ServiceContext, ServiceError, ServiceReply,
};
use emulator_memory::{BackingRange, Permissions};
const MAX_COMMAND: usize = 64 * 1024;
const MAX_DATA: usize = 4 * 1024 * 1024;
#[derive(Clone, Debug, Default)]
pub struct DiagnosticGpu {
    completed: u64,
}
#[derive(Debug)]
struct DmaEffect {
    submission: PreparedSubmission,
    writes: Vec<BackingRange>,
}
impl PreparedServiceEffect for DmaEffect {
    fn write_ranges(&self) -> &[BackingRange] {
        &self.writes
    }
    fn commit(self: Box<Self>) {
        let _ = self.submission.commit();
    }
}
fn device(error: impl std::fmt::Display) -> ServiceError {
    ServiceError::DeviceFault(error.to_string())
}
fn size(value: u64, limit: usize) -> Result<usize, ServiceError> {
    usize::try_from(value)
        .ok()
        .filter(|v| *v > 0 && *v <= limit)
        .ok_or_else(|| device("Diagnostic GPU range exceeds its positive size limit"))
}
impl Service for DiagnosticGpu {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            0 | 2 => Err(ServiceError::MissingMemoryContext),
            1 => {
                super::payload(request, 0)?;
                Ok(ServiceReply {
                    data: self.completed.to_le_bytes().to_vec(),
                    ..ServiceReply::default()
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "emu:gpu",
                command,
            }),
        }
    }
    fn prepare(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        context.require_copy_handles(0)?;
        if request.command_id == 2 {
            return self.buffered(request, context);
        }
        context.require_buffers(0, 0)?;
        if request.command_id != 0 {
            return self.dispatch(request);
        }
        super::payload(request, 64)?;
        let values: Vec<u64> = request
            .data
            .chunks_exact(8)
            .map(|b| u64::from_le_bytes(b.try_into().expect("eight bytes")))
            .collect();
        let command_size = size(values[2], MAX_COMMAND)?;
        let data_size = size(values[5], MAX_DATA)?;
        let entry_size = size(values[7], 256 * 8)?;
        if command_size % 4 != 0 || entry_size % 8 != 0 {
            return Err(device("Diagnostic GPU command/entry alignment"));
        }
        let commands = context.pin(values[0], command_size, Permissions::READ)?;
        let data = context.pin(values[3], data_size, Permissions::READ_WRITE)?;
        let entries = context.pin(values[6], entry_size, Permissions::READ)?;
        let mut bytes = vec![0; entry_size];
        entries.read(0, &mut bytes).map_err(ServiceError::Memory)?;
        let mut space = GpuAddressSpace::new(MAX_COMMAND + MAX_DATA).map_err(device)?;
        space.map(GpuVa(values[1]), commands).map_err(device)?;
        space.map(GpuVa(values[4]), data).map_err(device)?;
        self.finish(&space, &bytes)
    }
}
impl DiagnosticGpu {
    fn buffered(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        context.require_buffers(3, 1)?;
        super::payload(request, 24)?;
        let addresses: Vec<u64> = request
            .data
            .chunks_exact(8)
            .map(|b| u64::from_le_bytes(b.try_into().expect("eight bytes")))
            .collect();
        let commands = context.send_buffer(0)?;
        let source = context.send_buffer(1)?;
        let entries = context.send_buffer(2)?;
        let target = context.receive_buffer(0)?;
        size(commands.len() as u64, MAX_COMMAND)?;
        size(source.len() as u64, MAX_DATA)?;
        size(target.len() as u64, MAX_DATA)?;
        size(entries.len() as u64, 256 * 8)?;
        if commands.len() % 4 != 0 || entries.len() % 8 != 0 {
            return Err(device("Diagnostic GPU command/entry alignment"));
        }
        let mut bytes = vec![0; entries.len()];
        entries.read(0, &mut bytes).map_err(ServiceError::Memory)?;
        let mut space = GpuAddressSpace::new(MAX_COMMAND + MAX_DATA * 2).map_err(device)?;
        space.map(GpuVa(addresses[0]), commands).map_err(device)?;
        space.map(GpuVa(addresses[1]), source).map_err(device)?;
        space.map(GpuVa(addresses[2]), target).map_err(device)?;
        self.finish(&space, &bytes)
    }
    fn finish(
        &mut self,
        space: &GpuAddressSpace,
        bytes: &[u8],
    ) -> Result<ServiceReply, ServiceError> {
        let mut queue = SubmissionQueue::new(space, QueueLimits::default()).map_err(device)?;
        let id = queue.submit(space, bytes).map_err(device)?;
        let submission = queue
            .prepare_front(space, id, ExecutionLimits::default())
            .map_err(device)?;
        let stats = submission.stats();
        let writes = submission.write_ranges().collect();
        self.completed = self
            .completed
            .checked_add(1)
            .ok_or(ServiceError::ArithmeticOverflow)?;
        Ok(ServiceReply {
            data: [
                stats.methods as u64,
                stats.copy_launches as u64,
                stats.copied_bytes as u64,
                self.completed,
            ]
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect(),
            effect: Some(Box::new(DmaEffect { submission, writes })),
            ..ServiceReply::default()
        })
    }
}
