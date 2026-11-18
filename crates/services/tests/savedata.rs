//! Save data file systems through real CMIF requests: journaled commit,
//! read-only mounts, file locks, directory listings and rejections.
mod common;
use common::Rig;
use emulator_kernel::SVC_CLOSE_HANDLE;
use emulator_memory::Permissions;
use emulator_services::acc::DEFAULT_USER_ID;
use emulator_services::fspsrv::FspSrvService;
use emulator_services::savedata::*;

const PATHS: u64 = 0x40000;
const DATA: u64 = 0x50000;
const PROGRAM: u64 = 0x0100_0000_0000_1000;

fn rig() -> Rig {
    let mut rig = Rig::new("fsp-srv", Box::new(FspSrvService::default()));
    rig.m.map(PATHS, vec![0; 0x2000], Permissions::READ_WRITE).unwrap();
    rig.m.map(DATA, vec![0; 0x4000], Permissions::READ_WRITE).unwrap();
    rig
}

fn open_payload(save_type: u8, user: [u8; 16]) -> Vec<u8> {
    let mut data = vec![0u8; 0x48];
    data[0] = SAVE_DATA_SPACE_USER;
    data[8..16].copy_from_slice(&PROGRAM.to_le_bytes());
    data[16..32].copy_from_slice(&user);
    data[8 + 0x20] = save_type;
    data
}

/// Mounts the account save: 51 writable, 53 read-only.
fn mount(rig: &mut Rig, command: u32) -> u32 {
    rig.call(command, &[], &[], None, &open_payload(SAVE_DATA_TYPE_ACCOUNT, DEFAULT_USER_ID));
    assert_eq!(rig.handle_result(), 0);
    rig.moved()
}

/// X descriptors for `paths`, each NUL-terminated in its own 0x301 slot.
fn x(rig: &mut Rig, paths: &[&str]) -> Vec<(u64, u64)> {
    paths
        .iter()
        .enumerate()
        .map(|(slot, path)| {
            let address = PATHS + slot as u64 * 0x400;
            let mut bytes = path.as_bytes().to_vec();
            bytes.resize(0x301, 0);
            rig.m.write(address, &bytes).unwrap();
            (address, 0x301)
        })
        .collect()
}

/// IFileSystem command taking paths; returns the result.
fn fs(rig: &mut Rig, fs: u32, command: u32, paths: &[&str], data: &[u8]) -> u32 {
    let x = x(rig, paths);
    rig.x_call_on(fs, command, &x, &[], &[], data);
    rig.handle_result()
}

fn create_file(rig: &mut Rig, mount: u32, path: &str, size: i64) -> u32 {
    let mut data = vec![0u8; 16];
    data[8..].copy_from_slice(&size.to_le_bytes());
    fs(rig, mount, 0, &[path], &data)
}

fn open_file(rig: &mut Rig, mount: u32, path: &str, mode: u32) -> Result<u32, u32> {
    match fs(rig, mount, 8, &[path], &mode.to_le_bytes()) {
        0 => Ok(rig.moved()),
        result => Err(result),
    }
}

fn range(offset: i64, size: i64) -> Vec<u8> {
    let mut data = vec![0u8; 24];
    data[8..16].copy_from_slice(&offset.to_le_bytes());
    data[16..].copy_from_slice(&size.to_le_bytes());
    data
}

fn write(rig: &mut Rig, file: u32, offset: i64, bytes: &[u8]) -> u32 {
    rig.m.write(DATA, bytes).unwrap();
    rig.call_on(file, 1, &[(DATA, bytes.len() as u64)], &[], &range(offset, bytes.len() as i64));
    rig.result()
}

fn read(rig: &mut Rig, file: u32, offset: i64, size: i64, buffer: u64) -> (u32, Vec<u8>) {
    rig.m.write(DATA, &vec![0xee; buffer as usize]).unwrap();
    rig.call_on(file, 0, &[], &[(DATA, buffer)], &range(offset, size));
    if rig.result() != 0 {
        return (rig.result(), Vec::new());
    }
    let count = i64::from_le_bytes(rig.out(8).try_into().unwrap()) as usize;
    let mut bytes = vec![0u8; count];
    rig.m.read(DATA, &mut bytes).unwrap();
    (0, bytes)
}

fn entry_type(rig: &mut Rig, mount: u32, path: &str) -> Result<u32, u32> {
    match fs(rig, mount, 7, &[path], &[]) {
        0 => Ok(u32::from_le_bytes(rig.out(4).try_into().unwrap())),
        result => Err(result),
    }
}

fn close(rig: &mut Rig, handle: u32) {
    rig.c.x[0] = u64::from(handle);
    rig.k.dispatch(SVC_CLOSE_HANDLE, &mut rig.c, &mut rig.m).unwrap();
    assert_eq!(rig.c.x[0], 0);
}

#[test]
fn commit_publishes_writes_and_unmount_discards_the_rest() {
    let mut rig = rig();
    let writable = mount(&mut rig, 51);
    assert_eq!(entry_type(&mut rig, writable, "/"), Ok(ENTRY_TYPE_DIRECTORY));
    assert_eq!(create_file(&mut rig, writable, "/save.bin", 4), 0);
    let file = open_file(&mut rig, writable, "/save.bin", OPEN_MODE_WRITE | OPEN_MODE_APPEND).unwrap();
    assert_eq!(write(&mut rig, file, 2, b"0123456789abcdef"), 0);
    rig.call_on(file, 4, &[], &[], &[]);
    assert_eq!(rig.out(8), 18i64.to_le_bytes(), "appended past the end");

    // Not committed yet: a read-only mount sees nothing.
    let before = mount(&mut rig, 53);
    assert_eq!(entry_type(&mut rig, before, "/save.bin"), Err(RESULT_FS_PATH_NOT_FOUND));

    assert_eq!(fs(&mut rig, writable, 10, &[], &[]), 0);
    let after = mount(&mut rig, 53);
    let reader = open_file(&mut rig, after, "/save.bin", OPEN_MODE_READ).unwrap();
    let (result, bytes) = read(&mut rig, reader, 0, 64, 64);
    assert_eq!(result, 0);
    assert_eq!(bytes, b"\0\x000123456789abcdef");
    // The older read-only mount keeps its snapshot.
    assert_eq!(entry_type(&mut rig, before, "/save.bin"), Err(RESULT_FS_PATH_NOT_FOUND));

    // Read-only mounts refuse every mutation.
    assert_eq!(create_file(&mut rig, after, "/other", 0), RESULT_FS_UNSUPPORTED_OPERATION);
    assert_eq!(open_file(&mut rig, after, "/save.bin", OPEN_MODE_WRITE), Err(RESULT_FS_WRITE_NOT_PERMITTED));
    assert_eq!(fs(&mut rig, after, 10, &[], &[]), RESULT_FS_UNSUPPORTED_OPERATION);

    // Uncommitted changes vanish with their mount.
    assert_eq!(create_file(&mut rig, writable, "/scratch", 1), 0);
    close(&mut rig, file);
    close(&mut rig, writable);
    let again = mount(&mut rig, 51);
    assert_eq!(entry_type(&mut rig, again, "/save.bin"), Ok(ENTRY_TYPE_FILE));
    assert_eq!(entry_type(&mut rig, again, "/scratch"), Err(RESULT_FS_PATH_NOT_FOUND));
}

#[test]
fn open_files_lock_their_paths_until_closed() {
    let mut rig = rig();
    let save = mount(&mut rig, 51);
    assert_eq!(fs(&mut rig, save, 2, &["/dir"], &[]), 0);
    assert_eq!(create_file(&mut rig, save, "/dir/f", 8), 0);
    let first = open_file(&mut rig, save, "/dir/f", OPEN_MODE_READ).unwrap();
    // Readers share; a writer is exclusive.
    let second = open_file(&mut rig, save, "/dir/f", OPEN_MODE_READ).unwrap();
    assert_eq!(open_file(&mut rig, save, "/dir/f", OPEN_MODE_WRITE), Err(RESULT_FS_TARGET_LOCKED));
    assert_eq!(fs(&mut rig, save, 1, &["/dir/f"], &[]), RESULT_FS_TARGET_LOCKED);
    assert_eq!(fs(&mut rig, save, 5, &["/dir/f", "/g"], &[]), RESULT_FS_TARGET_LOCKED);
    assert_eq!(fs(&mut rig, save, 4, &["/dir"], &[]), RESULT_FS_TARGET_LOCKED);
    assert_eq!(fs(&mut rig, save, 6, &["/dir", "/moved"], &[]), RESULT_FS_TARGET_LOCKED);
    // A read handle cannot write.
    assert_eq!(write(&mut rig, first, 0, b"x"), RESULT_FS_WRITE_NOT_PERMITTED);

    close(&mut rig, first);
    assert_eq!(fs(&mut rig, save, 1, &["/dir/f"], &[]), RESULT_FS_TARGET_LOCKED, "one reader left");
    close(&mut rig, second);
    let writer = open_file(&mut rig, save, "/dir/f", OPEN_MODE_WRITE).unwrap();
    assert_eq!(open_file(&mut rig, save, "/dir/f", OPEN_MODE_READ), Err(RESULT_FS_TARGET_LOCKED));
    let (result, _) = read(&mut rig, writer, 0, 1, 1);
    assert_eq!(result, RESULT_FS_READ_NOT_PERMITTED);
    close(&mut rig, writer);
    assert_eq!(fs(&mut rig, save, 1, &["/dir/f"], &[]), 0);
    assert_eq!(entry_type(&mut rig, save, "/dir/f"), Err(RESULT_FS_PATH_NOT_FOUND));
}

#[test]
fn directories_list_rename_and_delete() {
    let mut rig = rig();
    let save = mount(&mut rig, 51);
    assert_eq!(fs(&mut rig, save, 2, &["/d"], &[]), 0);
    assert_eq!(fs(&mut rig, save, 2, &["/d/e"], &[]), 0);
    assert_eq!(create_file(&mut rig, save, "/d/a", 3), 0);
    assert_eq!(create_file(&mut rig, save, "/d/e/deep", 5), 0);
    assert_eq!(fs(&mut rig, save, 2, &["/d"], &[]), RESULT_FS_PATH_ALREADY_EXISTS);
    assert_eq!(fs(&mut rig, save, 3, &["/d"], &[]), RESULT_FS_DIRECTORY_NOT_EMPTY);

    assert_eq!(fs(&mut rig, save, 9, &["/d"], &(DIRECTORY_MODE_DIRECTORIES | DIRECTORY_MODE_FILES).to_le_bytes()), 0);
    let dir = rig.moved();
    rig.call_on(dir, 1, &[], &[], &[]);
    assert_eq!(rig.out(8), 2i64.to_le_bytes());
    let mut seen = Vec::new();
    loop {
        rig.call_on(dir, 0, &[], &[(DATA, DIRECTORY_ENTRY_SIZE as u64)], &[]);
        assert_eq!(rig.result(), 0);
        if rig.out(8) == 0i64.to_le_bytes() {
            break;
        }
        let mut entry = vec![0u8; DIRECTORY_ENTRY_SIZE];
        rig.m.read(DATA, &mut entry).unwrap();
        let name_end = entry.iter().position(|&b| b == 0).unwrap();
        let size = i64::from_le_bytes(entry[0x308..0x310].try_into().unwrap());
        seen.push((String::from_utf8(entry[..name_end].to_vec()).unwrap(), entry[0x304], size));
    }
    assert_eq!(seen, [("a".to_owned(), 1, 3), ("e".to_owned(), 0, 0)]);
    // Files only.
    assert_eq!(fs(&mut rig, save, 9, &["/d"], &DIRECTORY_MODE_FILES.to_le_bytes()), 0);
    let files = rig.moved();
    rig.call_on(files, 1, &[], &[], &[]);
    assert_eq!(rig.out(8), 1i64.to_le_bytes());

    assert_eq!(fs(&mut rig, save, 6, &["/d", "/d/e/inside"], &[]), RESULT_FS_INVALID_PATH_FORMAT);
    assert_eq!(fs(&mut rig, save, 6, &["/d", "/x"], &[]), 0);
    assert_eq!(entry_type(&mut rig, save, "/x/e/deep"), Ok(ENTRY_TYPE_FILE));
    assert_eq!(entry_type(&mut rig, save, "/d"), Err(RESULT_FS_PATH_NOT_FOUND));
    assert_eq!(fs(&mut rig, save, 5, &["/x/a", "/x/b"], &[]), 0);
    assert_eq!(fs(&mut rig, save, 5, &["/x/b", "/missing/b"], &[]), RESULT_FS_PATH_NOT_FOUND);
    assert_eq!(fs(&mut rig, save, 4, &["/"], &[]), RESULT_FS_TARGET_LOCKED, "root stays");
    assert_eq!(fs(&mut rig, save, 4, &["/x/e"], &[]), 0);
    assert_eq!(entry_type(&mut rig, save, "/x/b"), Ok(ENTRY_TYPE_FILE));
    assert_eq!(fs(&mut rig, save, 13, &["/"], &[]), 0);
    assert_eq!(entry_type(&mut rig, save, "/x"), Err(RESULT_FS_PATH_NOT_FOUND));
    assert_eq!(entry_type(&mut rig, save, "/"), Ok(ENTRY_TYPE_DIRECTORY));
}

#[test]
fn malformed_paths_ranges_and_saves_are_rejected() {
    let mut rig = rig();
    let save = mount(&mut rig, 51);
    for (path, result) in [
        ("relative", RESULT_FS_INVALID_PATH_FORMAT),
        ("/a//b", RESULT_FS_NOT_NORMALIZED),
        ("/a/../b", RESULT_FS_NOT_NORMALIZED),
        ("/./a", RESULT_FS_NOT_NORMALIZED),
        ("/missing/file", RESULT_FS_PATH_NOT_FOUND),
    ] {
        assert_eq!(create_file(&mut rig, save, path, 0), result, "{path}");
    }
    assert_eq!(create_file(&mut rig, save, "/f/", 2), 0, "trailing slash normalizes");
    assert_eq!(create_file(&mut rig, save, "/f", 0), RESULT_FS_PATH_ALREADY_EXISTS);
    assert_eq!(create_file(&mut rig, save, "/neg", -1), RESULT_FS_INVALID_SIZE);
    assert_eq!(create_file(&mut rig, save, "/huge", SAVE_DATA_CAPACITY as i64), RESULT_FS_USABLE_SPACE_NOT_ENOUGH);
    assert_eq!(open_file(&mut rig, save, "/f", 8), Err(RESULT_FS_INVALID_ARGUMENT));
    assert_eq!(open_file(&mut rig, save, "/f", OPEN_MODE_APPEND), Err(RESULT_FS_INVALID_ARGUMENT));

    let file = open_file(&mut rig, save, "/f", OPEN_MODE_READ | OPEN_MODE_WRITE).unwrap();
    assert_eq!(write(&mut rig, file, 1, b"xy"), RESULT_FS_FILE_EXTENSION_WITHOUT_APPEND);
    assert_eq!(write(&mut rig, file, 0, b"xy"), 0);
    assert_eq!(read(&mut rig, file, 3, 1, 1).0, RESULT_FS_OUT_OF_RANGE);
    assert_eq!(read(&mut rig, file, 2, 1, 1), (0, Vec::new()), "reading at the end returns nothing");
    assert_eq!(read(&mut rig, file, -1, 1, 1).0, RESULT_FS_INVALID_OFFSET);
    assert_eq!(read(&mut rig, file, 0, 8, 4).0, RESULT_FS_INVALID_SIZE, "size beyond the buffer");
    assert_eq!(read(&mut rig, file, 0, 8, 8), (0, b"xy".to_vec()));
    // SetSize grows with zeros and shrinks.
    rig.call_on(file, 3, &[], &[], &5i64.to_le_bytes());
    assert_eq!(read(&mut rig, file, 0, 8, 8), (0, b"xy\0\0\0".to_vec()));
    rig.call_on(file, 3, &[], &[], &1i64.to_le_bytes());
    assert_eq!(read(&mut rig, file, 0, 8, 8), (0, b"x".to_vec()));
    // A failed request (bad payload) leaves the file unchanged.
    rig.m.write(DATA, b"zz").unwrap();
    assert!(rig.try_call_on(file, 1, &[(DATA, 2)], &[], &[0; 8]).is_err());
    assert_eq!(read(&mut rig, file, 0, 8, 8), (0, b"x".to_vec()));

    // Space accounting covers growth through writes.
    assert_eq!(fs(&mut rig, save, 11, &["/"], &[]), 0);
    assert_eq!(rig.out(8), (SAVE_DATA_CAPACITY - 1).to_le_bytes());
    assert_eq!(fs(&mut rig, save, 12, &["/"], &[]), 0);
    assert_eq!(rig.out(8), SAVE_DATA_CAPACITY.to_le_bytes());

    // Unsupported commands, unknown users and other save kinds fail.
    assert!(rig.try_x_call_on(save, 14, &[], &[], &[], &[]).is_err());
    assert!(rig.try_x_call_on(save, 7, &[], &[], &[], &[]).is_err(), "path buffer required");
    let mut stranger = DEFAULT_USER_ID;
    stranger[3] = 9;
    rig.call(51, &[], &[], None, &open_payload(SAVE_DATA_TYPE_ACCOUNT, stranger));
    assert_eq!(rig.handle_result(), RESULT_FS_TARGET_NOT_FOUND);
    assert!(rig.try_call(51, &[], &[], None, &open_payload(0, [0; 16])).is_err(), "system save");
    assert!(rig.try_call(52, &[], &[], None, &open_payload(0, [0; 16])).is_err());
    assert!(rig.try_call(18, &[], &[], None, &[]).is_err(), "no SD card");
    // Device saves are separate from the account save.
    rig.call(51, &[], &[], None, &open_payload(SAVE_DATA_TYPE_DEVICE, [0; 16]));
    assert_eq!(rig.handle_result(), 0);
}
