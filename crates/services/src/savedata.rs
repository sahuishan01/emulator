//! Original save data file systems for `fsp-srv`.
//!
//! Each save (keyed by save data space and `SaveDataAttribute`) is a tree of
//! directories and files held in memory for the life of the emulated process.
//! Persistence across emulator runs is not implemented yet.
//!
//! Semantics follow Horizon's journaled save data:
//! - a writable mount works on a private copy of the committed tree; `Commit`
//!   publishes it, and unmounting without a commit discards the changes;
//! - a read-only mount sees the committed tree as of its opening;
//! - an open file locks its path: a writer is exclusive, and deleting or
//!   renaming an open file (or a directory containing one) is refused.
//!
//! Every mutation is validated while the request is prepared and applied
//! only when the reply commits, so a failed reply changes nothing.
//! Only account and device saves in the user space exist; other spaces and
//! save types, timestamps and QueryEntry fail explicitly.

use crate::acc::DEFAULT_USER_ID;
use crate::BufferWrite;
use emulator_ipc::Request;
use emulator_kernel::service::{
    PreparedServiceEffect, Service, ServiceContext, ServiceError, ServiceReply,
};
use emulator_memory::BackingRange;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;
use std::rc::Rc;

const fn fs(description: u32) -> u32 {
    2 | (description << 9)
}
pub const RESULT_FS_PATH_NOT_FOUND: u32 = fs(1);
pub const RESULT_FS_PATH_ALREADY_EXISTS: u32 = fs(2);
pub const RESULT_FS_TARGET_LOCKED: u32 = fs(7);
pub const RESULT_FS_DIRECTORY_NOT_EMPTY: u32 = fs(8);
pub const RESULT_FS_USABLE_SPACE_NOT_ENOUGH: u32 = fs(30);
pub const RESULT_FS_TARGET_NOT_FOUND: u32 = fs(1002);
pub const RESULT_FS_OUT_OF_RANGE: u32 = fs(3005);
pub const RESULT_FS_INVALID_ARGUMENT: u32 = fs(6001);
pub const RESULT_FS_TOO_LONG_PATH: u32 = fs(6003);
pub const RESULT_FS_INVALID_PATH_FORMAT: u32 = fs(6005);
pub const RESULT_FS_NOT_NORMALIZED: u32 = fs(6007);
pub const RESULT_FS_INVALID_OFFSET: u32 = fs(6061);
pub const RESULT_FS_INVALID_SIZE: u32 = fs(6062);
pub const RESULT_FS_FILE_EXTENSION_WITHOUT_APPEND: u32 = fs(6201);
pub const RESULT_FS_READ_NOT_PERMITTED: u32 = fs(6202);
pub const RESULT_FS_WRITE_NOT_PERMITTED: u32 = fs(6203);
pub const RESULT_FS_UNSUPPORTED_OPERATION: u32 = fs(6300);

/// Usable bytes per save (file contents only; the NACP is not parsed).
pub const SAVE_DATA_CAPACITY: u64 = 256 << 20;
/// Longest path, excluding the terminating NUL.
pub const MAX_PATH: usize = 0x300;
/// `DirectoryEntry`: name[0x301], attributes, pad, type at 0x304, size at 0x308.
pub const DIRECTORY_ENTRY_SIZE: usize = 0x310;

pub const SAVE_DATA_SPACE_USER: u8 = 1;
pub const SAVE_DATA_TYPE_ACCOUNT: u8 = 1;
pub const SAVE_DATA_TYPE_DEVICE: u8 = 3;

/// OpenFile mode bits.
pub const OPEN_MODE_READ: u32 = 1;
pub const OPEN_MODE_WRITE: u32 = 2;
pub const OPEN_MODE_APPEND: u32 = 4;
/// OpenDirectory mode bits.
pub const DIRECTORY_MODE_DIRECTORIES: u32 = 1;
pub const DIRECTORY_MODE_FILES: u32 = 2;
pub const DIRECTORY_MODE_NO_FILE_SIZE: u32 = 0x8000_0000;

/// Entry types returned by GetEntryType and in directory entries.
pub const ENTRY_TYPE_DIRECTORY: u32 = 0;
pub const ENTRY_TYPE_FILE: u32 = 1;

#[derive(Clone, Debug)]
enum Node {
    Directory,
    File(Vec<u8>),
}

/// A save's directories and files, keyed by normalized absolute path. The
/// root directory `/` is implicit.
#[derive(Clone, Debug, Default)]
pub struct Tree {
    entries: BTreeMap<String, Node>,
}

fn child_prefix(directory: &str) -> String {
    if directory == "/" {
        "/".to_owned()
    } else {
        format!("{directory}/")
    }
}

fn parent(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) | None => "/",
        Some(at) => &path[..at],
    }
}

impl Tree {
    fn is_directory(&self, path: &str) -> bool {
        path == "/" || matches!(self.entries.get(path), Some(Node::Directory))
    }

    fn file(&self, path: &str) -> Option<&Vec<u8>> {
        match self.entries.get(path) {
            Some(Node::File(data)) => Some(data),
            _ => None,
        }
    }

    fn exists(&self, path: &str) -> bool {
        path == "/" || self.entries.contains_key(path)
    }

    fn used(&self) -> u64 {
        self.entries
            .values()
            .map(|node| match node {
                Node::File(data) => data.len() as u64,
                Node::Directory => 0,
            })
            .sum()
    }

    /// Every path strictly below `directory`.
    fn descendants(&self, directory: &str) -> Vec<String> {
        let prefix = child_prefix(directory);
        self.entries
            .range(prefix.clone()..)
            .take_while(|(path, _)| path.starts_with(&prefix))
            .map(|(path, _)| path.clone())
            .collect()
    }

    /// Direct children of `directory`: (name, is_file, size).
    fn children(&self, directory: &str) -> Vec<(String, bool, u64)> {
        let prefix = child_prefix(directory);
        self.entries
            .range(prefix.clone()..)
            .take_while(|(path, _)| path.starts_with(&prefix))
            .filter_map(|(path, node)| {
                let name = &path[prefix.len()..];
                (!name.contains('/')).then(|| match node {
                    Node::File(data) => (name.to_owned(), true, data.len() as u64),
                    Node::Directory => (name.to_owned(), false, 0),
                })
            })
            .collect()
    }
}

/// Parses a NUL-terminated path from an X buffer into normalized form:
/// absolute, no empty, `.` or `..` components, no trailing slash.
pub fn parse_path(bytes: &[u8]) -> Result<String, u32> {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    if end > MAX_PATH {
        return Err(RESULT_FS_TOO_LONG_PATH);
    }
    let path = std::str::from_utf8(&bytes[..end]).map_err(|_| RESULT_FS_INVALID_PATH_FORMAT)?;
    if !path.starts_with('/') || path.contains(['\\', ':']) {
        return Err(RESULT_FS_INVALID_PATH_FORMAT);
    }
    if path == "/" {
        return Ok(path.to_owned());
    }
    let trimmed = path.strip_suffix('/').unwrap_or(path);
    if trimmed[1..].split('/').any(|part| part.is_empty() || part == "." || part == "..") {
        return Err(RESULT_FS_NOT_NORMALIZED);
    }
    Ok(trimmed.to_owned())
}

/// Committed trees of every save, keyed by space id and attribute.
pub type SaveDataStore = Rc<RefCell<BTreeMap<Vec<u8>, Tree>>>;

#[derive(Debug, Default)]
struct OpenCount {
    readers: usize,
    writers: usize,
}

#[derive(Debug)]
struct Mount {
    tree: Tree,
    writable: bool,
    store: SaveDataStore,
    key: Vec<u8>,
    open: BTreeMap<String, OpenCount>,
}

impl Mount {
    /// An open file at `path` or anywhere below it.
    fn locked(&self, path: &str) -> bool {
        let prefix = child_prefix(path);
        self.open.contains_key(path) || self.open.keys().any(|open| open.starts_with(&prefix))
    }
}

type SharedMount = Rc<RefCell<Mount>>;

/// Keeps a file's path locked while any copy of its IFile session lives.
#[derive(Debug)]
struct OpenGuard {
    mount: SharedMount,
    path: String,
    write: bool,
}

impl OpenGuard {
    fn new(mount: &SharedMount, path: &str, write: bool) -> Rc<Self> {
        let mut state = mount.borrow_mut();
        let count = state.open.entry(path.to_owned()).or_default();
        if write {
            count.writers += 1;
        } else {
            count.readers += 1;
        }
        Rc::new(Self { mount: mount.clone(), path: path.to_owned(), write })
    }
}

impl Drop for OpenGuard {
    fn drop(&mut self) {
        let mut state = self.mount.borrow_mut();
        if let Some(count) = state.open.get_mut(&self.path) {
            if self.write {
                count.writers -= 1;
            } else {
                count.readers -= 1;
            }
            if count.readers == 0 && count.writers == 0 {
                state.open.remove(&self.path);
            }
        }
    }
}

/// A validated mutation, applied when the reply commits.
struct Deferred(Box<dyn FnOnce()>);

impl fmt::Debug for Deferred {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Deferred")
    }
}

impl PreparedServiceEffect for Deferred {
    fn write_ranges(&self) -> &[BackingRange] {
        &[]
    }
    fn commit(self: Box<Self>) {
        (self.0)()
    }
}

fn deferred(apply: impl FnOnce() + 'static) -> ServiceReply {
    ServiceReply { effect: Some(Box::new(Deferred(Box::new(apply)))), ..ServiceReply::default() }
}

fn reply(data: Vec<u8>) -> ServiceReply {
    ServiceReply { data, ..ServiceReply::default() }
}

fn failure(result: u32) -> ServiceReply {
    ServiceReply { result, ..ServiceReply::default() }
}

fn le_u32(request: &Request, at: usize) -> u32 {
    u32::from_le_bytes(request.data[at..at + 4].try_into().expect("validated payload"))
}

fn le_i64(request: &Request, at: usize) -> i64 {
    i64::from_le_bytes(request.data[at..at + 8].try_into().expect("validated payload"))
}

/// Opens the save named by an `OpenSaveDataFileSystem` payload:
/// u8 space id, 7 bytes padding, 0x40-byte `SaveDataAttribute`.
pub fn open_save_data(store: &SaveDataStore, request: &Request, writable: bool) -> Result<ServiceReply, ServiceError> {
    crate::payload(request, 0x48)?;
    let space = request.data[0];
    let attribute = &request.data[8..0x48];
    let user: [u8; 16] = attribute[8..24].try_into().expect("16 bytes");
    let save_type = attribute[0x20];
    if space != SAVE_DATA_SPACE_USER {
        return Err(ServiceError::DeviceFault(format!("save data space {space} is not available")));
    }
    match save_type {
        SAVE_DATA_TYPE_ACCOUNT if user == DEFAULT_USER_ID => {}
        SAVE_DATA_TYPE_ACCOUNT => return Ok(failure(RESULT_FS_TARGET_NOT_FOUND)),
        SAVE_DATA_TYPE_DEVICE if user == [0; 16] => {}
        SAVE_DATA_TYPE_DEVICE => return Ok(failure(RESULT_FS_INVALID_ARGUMENT)),
        other => {
            return Err(ServiceError::DeviceFault(format!("save data type {other} is not available")));
        }
    }
    let mut key = vec![space];
    key.extend_from_slice(attribute);
    // An account or device save that was never committed is empty, as if
    // EnsureSaveData had just created it.
    let tree = store.borrow().get(&key).cloned().unwrap_or_default();
    let mount = Mount { tree, writable, store: store.clone(), key, open: BTreeMap::new() };
    Ok(ServiceReply {
        moved_objects: vec![Box::new(SaveFileSystem { mount: Rc::new(RefCell::new(mount)) })],
        ..ServiceReply::default()
    })
}

/// `IFileSystem` over one mounted save.
#[derive(Clone, Debug)]
pub struct SaveFileSystem {
    mount: SharedMount,
}

/// X buffers each IFileSystem command takes; `None` is unsupported.
fn path_count(command: u32) -> Option<usize> {
    match command {
        0..=4 | 7..=9 | 11..=13 => Some(1),
        5 | 6 => Some(2),
        10 => Some(0),
        _ => None,
    }
}

impl SaveFileSystem {
    fn command(&mut self, request: &Request, paths: &[String]) -> Result<ServiceReply, ServiceError> {
        let Some(count) = path_count(request.command_id) else {
            return Err(ServiceError::UnsupportedCommand { service: "IFileSystem (save data)", command: request.command_id });
        };
        if paths.len() != count {
            return Err(ServiceError::InvalidBufferCounts { send: 0, receive: 0 });
        }
        let mutating = matches!(request.command_id, 0..=6 | 10 | 13);
        let state = self.mount.borrow();
        if mutating && !state.writable {
            return Ok(failure(RESULT_FS_UNSUPPORTED_OPERATION));
        }
        let tree = &state.tree;
        let mount = self.mount.clone();
        match request.command_id {
            // CreateFile(u32 option, i64 size, X path)
            0 => {
                crate::payload(request, 16)?;
                let size = le_i64(request, 8);
                let path = paths[0].clone();
                let Ok(size) = u64::try_from(size) else {
                    return Ok(failure(RESULT_FS_INVALID_SIZE));
                };
                if tree.exists(&path) {
                    return Ok(failure(RESULT_FS_PATH_ALREADY_EXISTS));
                }
                if !tree.is_directory(parent(&path)) {
                    return Ok(failure(RESULT_FS_PATH_NOT_FOUND));
                }
                if tree.used().saturating_add(size) > SAVE_DATA_CAPACITY {
                    return Ok(failure(RESULT_FS_USABLE_SPACE_NOT_ENOUGH));
                }
                Ok(deferred(move || {
                    let file = Node::File(vec![0; size as usize]);
                    mount.borrow_mut().tree.entries.insert(path, file);
                }))
            }
            // DeleteFile(X path)
            1 => {
                crate::payload(request, 0)?;
                let path = paths[0].clone();
                if tree.file(&path).is_none() {
                    return Ok(failure(RESULT_FS_PATH_NOT_FOUND));
                }
                if state.locked(&path) {
                    return Ok(failure(RESULT_FS_TARGET_LOCKED));
                }
                Ok(deferred(move || {
                    mount.borrow_mut().tree.entries.remove(&path);
                }))
            }
            // CreateDirectory(X path)
            2 => {
                crate::payload(request, 0)?;
                let path = paths[0].clone();
                if tree.exists(&path) {
                    return Ok(failure(RESULT_FS_PATH_ALREADY_EXISTS));
                }
                if !tree.is_directory(parent(&path)) {
                    return Ok(failure(RESULT_FS_PATH_NOT_FOUND));
                }
                Ok(deferred(move || {
                    mount.borrow_mut().tree.entries.insert(path, Node::Directory);
                }))
            }
            // DeleteDirectory (3), DeleteDirectoryRecursively (4),
            // CleanDirectoryRecursively (13): the root can only be cleaned.
            3 | 4 | 13 => {
                crate::payload(request, 0)?;
                let path = paths[0].clone();
                if !tree.is_directory(&path) {
                    return Ok(failure(RESULT_FS_PATH_NOT_FOUND));
                }
                let keep_directory = request.command_id == 13;
                if path == "/" && !keep_directory {
                    return Ok(failure(RESULT_FS_TARGET_LOCKED));
                }
                let descendants = tree.descendants(&path);
                if request.command_id == 3 && !descendants.is_empty() {
                    return Ok(failure(RESULT_FS_DIRECTORY_NOT_EMPTY));
                }
                if state.locked(&path) {
                    return Ok(failure(RESULT_FS_TARGET_LOCKED));
                }
                Ok(deferred(move || {
                    let mut state = mount.borrow_mut();
                    for descendant in descendants {
                        state.tree.entries.remove(&descendant);
                    }
                    if !keep_directory {
                        state.tree.entries.remove(&path);
                    }
                }))
            }
            // RenameFile (5) / RenameDirectory (6)(X old, X new)
            5 | 6 => {
                crate::payload(request, 0)?;
                let (old, new) = (paths[0].clone(), paths[1].clone());
                let directory = request.command_id == 6;
                let found = if directory { old != "/" && tree.is_directory(&old) } else { tree.file(&old).is_some() };
                if !found {
                    return Ok(failure(RESULT_FS_PATH_NOT_FOUND));
                }
                if old == new {
                    return Ok(ServiceReply::default());
                }
                if tree.exists(&new) {
                    return Ok(failure(RESULT_FS_PATH_ALREADY_EXISTS));
                }
                if !tree.is_directory(parent(&new)) {
                    return Ok(failure(RESULT_FS_PATH_NOT_FOUND));
                }
                if directory && new.starts_with(&child_prefix(&old)) {
                    return Ok(failure(RESULT_FS_INVALID_PATH_FORMAT));
                }
                if state.locked(&old) {
                    return Ok(failure(RESULT_FS_TARGET_LOCKED));
                }
                let moved = if directory { tree.descendants(&old) } else { Vec::new() };
                Ok(deferred(move || {
                    let entries = &mut mount.borrow_mut().tree.entries;
                    for path in std::iter::once(old.clone()).chain(moved) {
                        let node = entries.remove(&path).expect("validated entry");
                        entries.insert(format!("{new}{}", &path[old.len()..]), node);
                    }
                }))
            }
            // GetEntryType(X path) -> u32
            7 => {
                crate::payload(request, 0)?;
                let path = &paths[0];
                let kind = if tree.is_directory(path) {
                    ENTRY_TYPE_DIRECTORY
                } else if tree.file(path).is_some() {
                    ENTRY_TYPE_FILE
                } else {
                    return Ok(failure(RESULT_FS_PATH_NOT_FOUND));
                };
                Ok(reply(kind.to_le_bytes().to_vec()))
            }
            // OpenFile(u32 mode, X path) -> IFile
            8 => {
                crate::payload(request, 4)?;
                let mode = le_u32(request, 0);
                let all = OPEN_MODE_READ | OPEN_MODE_WRITE | OPEN_MODE_APPEND;
                if mode & !all != 0 || mode & (OPEN_MODE_READ | OPEN_MODE_WRITE) == 0 {
                    return Ok(failure(RESULT_FS_INVALID_ARGUMENT));
                }
                let write = mode & (OPEN_MODE_WRITE | OPEN_MODE_APPEND) != 0;
                if write && !state.writable {
                    return Ok(failure(RESULT_FS_WRITE_NOT_PERMITTED));
                }
                let path = &paths[0];
                if tree.file(path).is_none() {
                    return Ok(failure(RESULT_FS_PATH_NOT_FOUND));
                }
                if let Some(count) = state.open.get(path) {
                    if count.writers > 0 || write {
                        return Ok(failure(RESULT_FS_TARGET_LOCKED));
                    }
                }
                drop(state);
                let guard = OpenGuard::new(&self.mount, path, write);
                Ok(ServiceReply {
                    moved_objects: vec![Box::new(SaveFile { guard, mode })],
                    ..ServiceReply::default()
                })
            }
            // OpenDirectory(u32 mode, X path) -> IDirectory (a snapshot).
            9 => {
                crate::payload(request, 4)?;
                let mode = le_u32(request, 0);
                let all = DIRECTORY_MODE_DIRECTORIES | DIRECTORY_MODE_FILES | DIRECTORY_MODE_NO_FILE_SIZE;
                if mode & !all != 0 {
                    return Ok(failure(RESULT_FS_INVALID_ARGUMENT));
                }
                if !tree.is_directory(&paths[0]) {
                    return Ok(failure(RESULT_FS_PATH_NOT_FOUND));
                }
                let entries = tree
                    .children(&paths[0])
                    .into_iter()
                    .filter(|(_, is_file, _)| {
                        mode & if *is_file { DIRECTORY_MODE_FILES } else { DIRECTORY_MODE_DIRECTORIES } != 0
                    })
                    .map(|(name, is_file, size)| {
                        let mut entry = vec![0u8; DIRECTORY_ENTRY_SIZE];
                        entry[..name.len()].copy_from_slice(name.as_bytes());
                        entry[0x304] = if is_file { ENTRY_TYPE_FILE as u8 } else { ENTRY_TYPE_DIRECTORY as u8 };
                        let size = if mode & DIRECTORY_MODE_NO_FILE_SIZE != 0 { 0 } else { size };
                        entry[0x308..0x310].copy_from_slice(&size.to_le_bytes());
                        entry
                    })
                    .collect();
                Ok(ServiceReply {
                    moved_objects: vec![Box::new(SaveDirectory { entries, next: 0 })],
                    ..ServiceReply::default()
                })
            }
            // Commit: publish this mount's tree as the save's committed state.
            10 => {
                crate::payload(request, 0)?;
                let (store, key, snapshot) = (state.store.clone(), state.key.clone(), tree.clone());
                Ok(deferred(move || {
                    store.borrow_mut().insert(key, snapshot);
                }))
            }
            // GetFreeSpaceSize (11) / GetTotalSpaceSize (12)(X path) -> i64
            11 | 12 => {
                crate::payload(request, 0)?;
                if !tree.exists(&paths[0]) {
                    return Ok(failure(RESULT_FS_PATH_NOT_FOUND));
                }
                let size = if request.command_id == 11 { SAVE_DATA_CAPACITY - tree.used() } else { SAVE_DATA_CAPACITY };
                Ok(reply(size.to_le_bytes().to_vec()))
            }
            _ => unreachable!("path_count covers every supported command"),
        }
    }
}

impl Service for SaveFileSystem {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn prepare(&mut self, request: &Request, context: &ServiceContext<'_>) -> Result<ServiceReply, ServiceError> {
        context.require_copy_handles(0)?;
        let Some(count) = path_count(request.command_id) else {
            return self.command(request, &[]);
        };
        context.require_exact_buffers(0, 0, count, 0)?;
        let mut paths = Vec::with_capacity(count);
        for index in 0..count {
            let mut bytes = Vec::new();
            if let Some(view) = context.pointer_send(index)? {
                bytes = vec![0u8; view.len().min(MAX_PATH + 1)];
                view.read(0, &mut bytes).map_err(ServiceError::Memory)?;
            }
            match parse_path(&bytes) {
                Ok(path) => paths.push(path),
                Err(result) => return Ok(failure(result)),
            }
        }
        self.command(request, &paths)
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        self.command(request, &[])
    }
}

/// `IFile` in a mounted save. Reads and writes go to the mount's tree.
#[derive(Clone, Debug)]
pub struct SaveFile {
    guard: Rc<OpenGuard>,
    mode: u32,
}

/// Validated (offset, size) of a Read/Write payload:
/// u32 option, 4 bytes padding, i64 offset, i64 size.
fn file_range(request: &Request, buffer: usize) -> Result<Result<(u64, u64), u32>, ServiceError> {
    crate::payload(request, 24)?;
    let (offset, size) = (le_i64(request, 8), le_i64(request, 16));
    let Ok(offset) = u64::try_from(offset) else {
        return Ok(Err(RESULT_FS_INVALID_OFFSET));
    };
    match u64::try_from(size) {
        Ok(size) if size <= buffer as u64 && offset.checked_add(size).is_some() => Ok(Ok((offset, size))),
        _ => Ok(Err(RESULT_FS_INVALID_SIZE)),
    }
}

impl SaveFile {
    fn length(&self) -> Result<u64, ServiceError> {
        let mount = self.guard.mount.borrow();
        mount
            .tree
            .file(&self.guard.path)
            .map(|data| data.len() as u64)
            .ok_or_else(|| ServiceError::DeviceFault(format!("open save file {} vanished", self.guard.path)))
    }

    /// Space check for growing this file to `length` bytes.
    fn fits(&self, length: u64) -> Result<bool, ServiceError> {
        let current = self.length()?;
        let used = self.guard.mount.borrow().tree.used();
        Ok(length <= current || used - current + length <= SAVE_DATA_CAPACITY)
    }

    fn resize_and_write(&self, offset: u64, bytes: Vec<u8>, set_size: Option<u64>) -> ServiceReply {
        let mount = self.guard.mount.clone();
        let path = self.guard.path.clone();
        deferred(move || {
            let mut state = mount.borrow_mut();
            let Some(Node::File(data)) = state.tree.entries.get_mut(&path) else {
                unreachable!("open files cannot be removed");
            };
            if let Some(length) = set_size {
                data.resize(length as usize, 0);
            }
            let end = offset as usize + bytes.len();
            if end > data.len() {
                data.resize(end, 0);
            }
            data[offset as usize..end].copy_from_slice(&bytes);
        })
    }
}

impl Service for SaveFile {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn prepare(&mut self, request: &Request, context: &ServiceContext<'_>) -> Result<ServiceReply, ServiceError> {
        context.require_copy_handles(0)?;
        match request.command_id {
            // Read(u32 option, i64 offset, i64 size, B buffer) -> i64 read
            0 => {
                context.require_exact_buffers(0, 1, 0, 0)?;
                let view = context.receive_buffer(0)?;
                let (offset, size) = match file_range(request, view.len())? {
                    Ok(range) => range,
                    Err(result) => return Ok(failure(result)),
                };
                if self.mode & OPEN_MODE_READ == 0 {
                    return Ok(failure(RESULT_FS_READ_NOT_PERMITTED));
                }
                let length = self.length()?;
                if offset > length {
                    return Ok(failure(RESULT_FS_OUT_OF_RANGE));
                }
                let count = size.min(length - offset) as usize;
                let mut reply = reply((count as i64).to_le_bytes().to_vec());
                if count > 0 {
                    let mount = self.guard.mount.borrow();
                    let data = mount.tree.file(&self.guard.path).expect("checked above");
                    let bytes = data[offset as usize..offset as usize + count].to_vec();
                    let view = view.slice(0, count).map_err(ServiceError::Memory)?;
                    reply.effect = Some(Box::new(BufferWrite::new(view, bytes)));
                }
                Ok(reply)
            }
            // Write(u32 option, i64 offset, i64 size, A buffer)
            1 => {
                context.require_exact_buffers(1, 0, 0, 0)?;
                let view = context.send_buffer(0)?;
                let (offset, size) = match file_range(request, view.len())? {
                    Ok(range) => range,
                    Err(result) => return Ok(failure(result)),
                };
                if self.mode & OPEN_MODE_WRITE == 0 {
                    return Ok(failure(RESULT_FS_WRITE_NOT_PERMITTED));
                }
                let end = offset + size;
                if end > self.length()? && self.mode & OPEN_MODE_APPEND == 0 {
                    return Ok(failure(RESULT_FS_FILE_EXTENSION_WITHOUT_APPEND));
                }
                if !self.fits(end)? {
                    return Ok(failure(RESULT_FS_USABLE_SPACE_NOT_ENOUGH));
                }
                let mut bytes = vec![0u8; size as usize];
                view.read(0, &mut bytes).map_err(ServiceError::Memory)?;
                Ok(self.resize_and_write(offset, bytes, None))
            }
            _ => {
                context.require_buffers(0, 0)?;
                self.dispatch(request)
            }
        }
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // Flush: writes are already in the mount; Commit makes them durable.
            2 => {
                crate::payload(request, 0)?;
                Ok(ServiceReply::default())
            }
            // SetSize(i64)
            3 => {
                crate::payload(request, 8)?;
                let Ok(size) = u64::try_from(le_i64(request, 0)) else {
                    return Ok(failure(RESULT_FS_INVALID_SIZE));
                };
                if self.mode & OPEN_MODE_WRITE == 0 {
                    return Ok(failure(RESULT_FS_WRITE_NOT_PERMITTED));
                }
                if !self.fits(size)? {
                    return Ok(failure(RESULT_FS_USABLE_SPACE_NOT_ENOUGH));
                }
                Ok(self.resize_and_write(0, Vec::new(), Some(size)))
            }
            // GetSize -> i64
            4 => {
                crate::payload(request, 0)?;
                Ok(reply((self.length()? as i64).to_le_bytes().to_vec()))
            }
            command => Err(ServiceError::UnsupportedCommand { service: "IFile (save data)", command }),
        }
    }
}

/// `IDirectory`: entries as of OpenDirectory, read in order.
#[derive(Clone, Debug)]
pub struct SaveDirectory {
    entries: Vec<Vec<u8>>,
    next: usize,
}

impl Service for SaveDirectory {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn prepare(&mut self, request: &Request, context: &ServiceContext<'_>) -> Result<ServiceReply, ServiceError> {
        context.require_copy_handles(0)?;
        if request.command_id != 0 {
            context.require_buffers(0, 0)?;
            return self.dispatch(request);
        }
        // Read(B buffer of DirectoryEntry) -> i64 count
        crate::payload(request, 0)?;
        context.require_exact_buffers(0, 1, 0, 0)?;
        let view = context.receive_buffer(0)?;
        let count = (view.len() / DIRECTORY_ENTRY_SIZE).min(self.entries.len() - self.next);
        let bytes: Vec<u8> = self.entries[self.next..self.next + count].concat();
        self.next += count;
        let mut reply = reply((count as i64).to_le_bytes().to_vec());
        if count > 0 {
            let view = view.slice(0, bytes.len()).map_err(ServiceError::Memory)?;
            reply.effect = Some(Box::new(BufferWrite::new(view, bytes)));
        }
        Ok(reply)
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetEntryCount -> i64 (all entries, read or not)
            1 => {
                crate::payload(request, 0)?;
                Ok(reply((self.entries.len() as i64).to_le_bytes().to_vec()))
            }
            command => Err(ServiceError::UnsupportedCommand { service: "IDirectory (save data)", command }),
        }
    }
}
