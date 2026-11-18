//! Original `bcat:u` (background content delivery) service for an offline
//! console: the application's delivery cache exists but nothing has ever been
//! delivered, so it holds no directories. Synchronisation needs a delivery
//! server and fails explicitly, as do file and directory opens (there is
//! nothing to open) and access to other applications' caches.

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceContext, ServiceError, ServiceReply};

use crate::payload;

/// `IServiceCreator`.
#[derive(Clone, Debug, Default)]
pub struct BcatServiceCreator;

impl Service for BcatServiceCreator {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // CreateDeliveryCacheStorageService(process id): the caller's own cache.
            1 => {
                payload(request, 8)?;
                if request.pid.is_none() {
                    return Err(ServiceError::MissingSenderIdentity);
                }
                Ok(ServiceReply {
                    moved_objects: vec![Box::new(DeliveryCacheStorage)],
                    ..ServiceReply::default()
                })
            }
            // CreateBcatService (0) only drives network synchronisation;
            // other applications' caches (2) are not reachable.
            command => Err(ServiceError::UnsupportedCommand { service: "bcat:u", command }),
        }
    }
}

/// `IDeliveryCacheStorageService` over an empty delivery cache.
#[derive(Clone, Debug, Default)]
pub struct DeliveryCacheStorage;

impl Service for DeliveryCacheStorage {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn prepare(&mut self, request: &Request, context: &ServiceContext<'_>) -> Result<ServiceReply, ServiceError> {
        context.require_copy_handles(0)?;
        if request.command_id == 10 {
            // EnumerateDeliveryCacheDirectory(B: DirectoryName[]) -> i32 count.
            // No directories: the output buffer is left untouched.
            payload(request, 0)?;
            context.require_exact_buffers(0, 1, 0, 0)?;
            return Ok(ServiceReply { data: 0i32.to_le_bytes().to_vec(), ..ServiceReply::default() });
        }
        context.require_buffers(0, 0)?;
        self.dispatch(request)
    }
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        // CreateFileService (0) / CreateDirectoryService (1) could only open
        // entries of an empty cache; not modelled until content exists.
        Err(ServiceError::UnsupportedCommand {
            service: "IDeliveryCacheStorageService",
            command: request.command_id,
        })
    }
}
