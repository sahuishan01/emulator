//! Original `fsp-srv` (Filesystem Proxy Service) implementation.
//! Provides filesystem handles, RomFS and savedata mount interfaces.

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceError, ServiceReply};

#[derive(Clone, Debug, Default)]
pub struct FspSrvService;

#[derive(Clone, Debug, Default)]
pub struct FileSystemProxy;

impl Service for FileSystemProxy {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // OpenFile / OpenDirectory: placeholder successful stub for initial filesystem inquiries
            0 | 1 => {
                Ok(ServiceReply {
                    result: 0,
                    data: vec![0; 8],
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "IFileSystem",
                command,
            }),
        }
    }
}

impl Service for FspSrvService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // Initialize: Set program identity / process ID
            1 => {
                Ok(ServiceReply {
                    result: 0,
                    data: Vec::new(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // OpenDataFileSystemByCurrentProcess (RomFS mount)
            // Returns an IFileSystem session object
            2 => {
                Ok(ServiceReply {
                    result: 0,
                    data: Vec::new(),
                    moved_objects: vec![Box::new(FileSystemProxy::default())],
                    effect: None,
                })
            }
            // OpenSaveDataFileSystem: returns IFileSystem session object
            18 => {
                Ok(ServiceReply {
                    result: 0,
                    data: Vec::new(),
                    moved_objects: vec![Box::new(FileSystemProxy::default())],
                    effect: None,
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "fsp-srv",
                command,
            }),
        }
    }
}
