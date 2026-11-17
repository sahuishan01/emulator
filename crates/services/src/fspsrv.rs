//! Original `fsp-srv` (Filesystem Proxy Service) implementation.
//! Provides filesystem handles, RomFS and savedata mount interfaces.

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceContext, ServiceError, ServiceReply};
use emulator_loader::RomFsArchive;

/// fs result: access outside a storage (module 2, desc 3005, OutOfRange).
pub const RESULT_FS_OUT_OF_RANGE: u32 = (3005 << 9) | 2;
/// fs result: operation not supported by this storage (module 2, desc 6300).
pub const RESULT_FS_UNSUPPORTED_WRITE: u32 = (6300 << 9) | 2;

/// IStorage over the program's data storage (OpenDataStorageByCurrentProcess).
/// Read-only: Write and SetSize fail explicitly.
#[derive(Clone, Debug)]
pub struct StorageProxy {
    storage: crate::DataStorage,
}

impl StorageProxy {
    pub fn new(storage: crate::DataStorage) -> Self {
        Self { storage }
    }
}

/// Largest single IStorage::Read served (bounds host allocation per request).
pub const MAX_STORAGE_READ: u64 = 64 * 1024 * 1024;

fn result(code: u32) -> ServiceReply {
    ServiceReply {
        result: code,
        ..ServiceReply::default()
    }
}

impl Service for StorageProxy {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // Read needs its output buffer: see `prepare`.
            0 => Err(ServiceError::MissingMemoryContext),
            // Write / SetSize on read-only storage
            1 | 3 => Ok(result(RESULT_FS_UNSUPPORTED_WRITE)),
            // Flush
            2 => {
                crate::payload(request, 0)?;
                Ok(ServiceReply::default())
            }
            // GetSize
            4 => {
                crate::payload(request, 0)?;
                Ok(ServiceReply {
                    data: self.storage.size().to_le_bytes().to_vec(),
                    ..ServiceReply::default()
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "IStorage",
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
        if request.command_id != 0 {
            context.require_buffers(0, 0)?;
            return self.dispatch(request);
        }
        // Read(u64 offset, u64 size, B buffer)
        crate::payload(request, 16)?;
        context.require_buffers(0, 1)?;
        let offset = u64::from_le_bytes(request.data[0..8].try_into().unwrap());
        let size = u64::from_le_bytes(request.data[8..16].try_into().unwrap());
        let output = context.receive_buffer(0)?;
        let end = offset.checked_add(size);
        if size > output.len() as u64 || end.is_none_or(|end| end > self.storage.size()) {
            return Ok(result(RESULT_FS_OUT_OF_RANGE));
        }
        if size > MAX_STORAGE_READ {
            return Err(ServiceError::BufferLimit);
        }
        let mut reply = ServiceReply::default();
        if size != 0 {
            let mut bytes = vec![0; size as usize];
            self.storage
                .read(offset, &mut bytes)
                .map_err(|error| ServiceError::DeviceFault(format!("data storage: {error:?}")))?;
            let mut view = output;
            if view.len() as u64 != size {
                view = view.slice(0, size as usize).map_err(ServiceError::Memory)?;
            }
            reply.effect = Some(Box::new(crate::BufferWrite::new(view, bytes)));
        }
        Ok(reply)
    }
}

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
pub struct DirectoryEntry {
    pub name: String,
    pub is_file: bool,
    pub size: u64,
}

#[derive(Clone, Debug, Default)]
pub struct DirectoryProxy {
    entries: Vec<DirectoryEntry>,
    read_index: usize,
}

impl Service for DirectoryProxy {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // Read (Command 0) -> returns u64 count, plus entries buffer
            0 => {
                let remaining = self.entries.len().saturating_sub(self.read_index);
                let count = remaining.min(32);
                let mut data = (count as u64).to_le_bytes().to_vec();
                for i in 0..count {
                    let entry = &self.entries[self.read_index + i];
                    let mut entry_bytes = vec![0u8; 0x310]; // 784 bytes per DirEntry
                    let name_bytes = entry.name.as_bytes();
                    let len = name_bytes.len().min(768);
                    entry_bytes[..len].copy_from_slice(&name_bytes[..len]);
                    entry_bytes[0x303] = if entry.is_file { 1 } else { 0 }; // entry_type
                    entry_bytes[0x308..0x310].copy_from_slice(&entry.size.to_le_bytes());
                    data.extend_from_slice(&entry_bytes);
                }
                self.read_index += count;
                Ok(ServiceReply {
                    result: 0,
                    data,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // GetEntryCount (Command 1) -> returns u64 total count
            1 => {
                let count = self.entries.len() as u64;
                Ok(ServiceReply {
                    result: 0,
                    data: count.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "IDirectory",
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
                let path_str = std::str::from_utf8(&request.data)
                    .unwrap_or("")
                    .trim_matches('\0');
                let mut entries = Vec::new();
                if let Some(ref archive) = self.romfs {
                    for file_name in archive.file_paths() {
                        let trimmed = file_name.trim_start_matches('/');
                        let dir_prefix = path_str.trim_matches('/');
                        if dir_prefix.is_empty() || trimmed.starts_with(dir_prefix) {
                            let sub = if dir_prefix.is_empty() {
                                trimmed
                            } else {
                                trimmed.strip_prefix(dir_prefix).unwrap_or(trimmed).trim_start_matches('/')
                            };
                            let item_name = sub.split('/').next().unwrap_or(sub);
                            if !item_name.is_empty() && !entries.iter().any(|e: &DirectoryEntry| e.name == item_name) {
                                let is_file = !sub.contains('/');
                                let size = if is_file {
                                    archive.get_file(&file_name).map(|b| b.len() as u64).unwrap_or(0)
                                } else {
                                    0
                                };
                                entries.push(DirectoryEntry {
                                    name: item_name.to_string(),
                                    is_file,
                                    size,
                                });
                            }
                        }
                    }
                }
                Ok(ServiceReply {
                    result: 0,
                    data: vec![0; 8],
                    moved_objects: vec![Box::new(DirectoryProxy {
                        entries,
                        read_index: 0,
                    })],
                    effect: None,
                })
            }
            // CreateFile (Command 8)
            8 => Ok(ServiceReply::default()),
            // DeleteFile (Command 9)
            9 => Ok(ServiceReply::default()),
            // Commit (Command 10)
            10 => Ok(ServiceReply::default()),
            // CreateDirectory (Command 11)
            11 => Ok(ServiceReply::default()),
            // DeleteDirectory (Command 12)
            12 => Ok(ServiceReply::default()),
            // GetFreeSpaceSize (Command 49)
            49 => {
                let free_space: u64 = 32 * 1024 * 1024 * 1024; // 32 GiB
                Ok(ServiceReply {
                    result: 0,
                    data: free_space.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // GetTotalSpaceSize (Command 50)
            50 => {
                let total_space: u64 = 64 * 1024 * 1024 * 1024; // 64 GiB
                Ok(ServiceReply {
                    result: 0,
                    data: total_space.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // CleanDirectoryRecursively (Command 51)
            51 => Ok(ServiceReply::default()),
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
    data_storage: Option<crate::DataStorage>,
}

impl FspSrvService {
    pub fn with_romfs(romfs: Option<RomFsArchive>) -> Self {
        let data_storage = romfs.clone().map(|romfs| std::rc::Rc::new(romfs) as crate::DataStorage);
        Self { romfs, data_storage }
    }

    pub fn with_storage(romfs: Option<RomFsArchive>, data_storage: Option<crate::DataStorage>) -> Self {
        Self { romfs, data_storage }
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
            18 | 51 | 52 => {
                Ok(ServiceReply {
                    result: 0,
                    data: Vec::new(),
                    moved_objects: vec![Box::new(FileSystemProxy::default())],
                    effect: None,
                })
            }
            // OpenDataStorageByCurrentProcess -> IStorage over the RomFS image.
            // Other titles' data storages are not available and fail explicitly.
            200 => {
                crate::payload(request, 0)?;
                let storage = self.data_storage.clone().ok_or(ServiceError::UnsupportedCommand {
                    service: "fsp-srv (no data storage loaded)",
                    command: 200,
                })?;
                Ok(ServiceReply {
                    moved_objects: vec![Box::new(StorageProxy::new(storage))],
                    ..ServiceReply::default()
                })
            }
            // SetGlobalAccessLogMode(u32), OutputAccessLogToSdCard(buffer)
            1004 | 1006 => Ok(ServiceReply::default()),
            // GetGlobalAccessLogMode: logging disabled
            1005 => {
                crate::payload(request, 0)?;
                Ok(ServiceReply {
                    data: 0u32.to_le_bytes().to_vec(),
                    ..ServiceReply::default()
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "fsp-srv",
                command,
            }),
        }
    }
}
