//! Original `fsp-srv` (Filesystem Proxy Service) implementation.
//! Provides filesystem handles, RomFS and savedata mount interfaces.

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceError, ServiceReply};
use emulator_loader::RomFsArchive;

#[derive(Clone, Debug, Default)]
pub struct FileProxy {
    file_bytes: Vec<u8>,
}

impl Service for FileProxy {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // Read (Command 0): offset: u64, size: u64 -> out bytes
            0 => {
                let offset = if request.data.len() >= 8 {
                    u64::from_le_bytes(request.data[0..8].try_into().unwrap()) as usize
                } else {
                    0
                };
                let size = if request.data.len() >= 16 {
                    u64::from_le_bytes(request.data[8..16].try_into().unwrap()) as usize
                } else {
                    self.file_bytes.len().saturating_sub(offset)
                };

                let start = offset.min(self.file_bytes.len());
                let end = (start + size).min(self.file_bytes.len());
                let read_bytes = self.file_bytes[start..end].to_vec();

                Ok(ServiceReply {
                    result: 0,
                    data: read_bytes,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // GetSize (Command 4): returns u64 size
            4 => {
                let size = self.file_bytes.len() as u64;
                Ok(ServiceReply {
                    result: 0,
                    data: size.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "IFile",
                command,
            }),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct FileSystemProxy {
    romfs: Option<RomFsArchive>,
}

impl FileSystemProxy {
    pub fn new(romfs: Option<RomFsArchive>) -> Self {
        Self { romfs }
    }
}

impl Service for FileSystemProxy {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // OpenFile (Command 0): opens file, returns IFile
            0 => {
                let path_str = std::str::from_utf8(&request.data)
                    .unwrap_or("")
                    .trim_matches('\0');
                let file_bytes = if let Some(ref archive) = self.romfs {
                    archive.get_file(path_str).map(|s| s.to_vec()).unwrap_or_default()
                } else {
                    Vec::new()
                };

                Ok(ServiceReply {
                    result: 0,
                    data: vec![0; 8],
                    moved_objects: vec![Box::new(FileProxy { file_bytes })],
                    effect: None,
                })
            }
            // OpenDirectory (Command 1)
            1 => {
                Ok(ServiceReply {
                    result: 0,
                    data: vec![0; 8],
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // GetEntryType (Command 7): 1 = Directory, 2 = File
            7 => {
                let path_str = std::str::from_utf8(&request.data)
                    .unwrap_or("")
                    .trim_matches('\0');
                let is_file = self.romfs.as_ref().map(|a| a.contains_file(path_str)).unwrap_or(false);
                let entry_type: u32 = if is_file { 2 } else { 1 };
                Ok(ServiceReply {
                    result: 0,
                    data: entry_type.to_le_bytes().to_vec(),
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

#[derive(Clone, Debug, Default)]
pub struct FspSrvService {
    romfs: Option<RomFsArchive>,
}

impl FspSrvService {
    pub fn with_romfs(romfs: Option<RomFsArchive>) -> Self {
        Self { romfs }
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
                    moved_objects: vec![Box::new(FileSystemProxy::new(self.romfs.clone()))],
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
