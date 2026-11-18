//! Original `aoc:u` (add-on content) service for a title with no add-on
//! content installed: counts are zero, lists are empty and the change event
//! is never signaled.

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceContext, ServiceError, ServiceReply};

use crate::event::EventHandle;
use crate::payload;

#[derive(Clone, Debug, Default)]
pub struct AddOnContentManager;

impl Service for AddOnContentManager {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // CountAddOnContent(process id) -> u32
            2 => {
                payload(request, 8)?;
                if request.pid.is_none() {
                    return Err(ServiceError::MissingSenderIdentity);
                }
                Ok(ServiceReply {
                    data: 0_u32.to_le_bytes().to_vec(),
                    ..ServiceReply::default()
                })
            }
            // CheckAddOnContentMountStatus(process id): nothing is installed,
            // so no mounted add-on content can have been lost.
            50 => {
                payload(request, 8)?;
                if request.pid.is_none() {
                    return Err(ServiceError::MissingSenderIdentity);
                }
                Ok(ServiceReply::default())
            }
            // GetAddOnContentListChangedEvent (8) and its process-id variant (10).
            8 | 10 => {
                payload(request, if request.command_id == 8 { 0 } else { 8 })?;
                Ok(ServiceReply {
                    moved_objects: vec![Box::new(EventHandle::new(false))],
                    ..ServiceReply::default()
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "aoc:u",
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
        if request.command_id != 3 {
            context.require_buffers(0, 0)?;
            return self.dispatch(request);
        }
        // ListAddOnContent(u32 offset, u32 count, process id) -> u32 written,
        // into a u32 index buffer that stays untouched for an empty list.
        payload(request, 16)?;
        if request.pid.is_none() {
            return Err(ServiceError::MissingSenderIdentity);
        }
        context.require_buffers(0, 1)?;
        Ok(ServiceReply {
            data: 0_u32.to_le_bytes().to_vec(),
            ..ServiceReply::default()
        })
    }
}
