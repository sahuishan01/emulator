//! Bounded value-only uploads and resumable execution of user-selected static NROs.
use crate::{quote, EngineReport, REPORT_CAPACITY};
use emulator_loader::NroImage;
use emulator_session::{RunOutcome, Session};
use std::cell::RefCell;
const MAX_FILE: usize = 32 * 1024 * 1024;
const MAX_INSTRUCTIONS: u64 = 2_000_000;
const CATCH: &[u8] = include_bytes!("../../../examples/homebrew/catch.nro");

#[repr(C)]
pub struct ProgramChunk {
    pub length: u32,
    pub bytes: [u8; 4096],
}

#[cfg(unix)]
struct MmapFile {
    ptr: *mut std::ffi::c_void,
    len: usize,
}

#[cfg(unix)]
unsafe impl Send for MmapFile {}
#[cfg(unix)]
unsafe impl Sync for MmapFile {}

#[cfg(unix)]
impl MmapFile {
    fn open(path: &std::path::Path) -> std::io::Result<Self> {
        use std::os::unix::io::AsRawFd;

        let file = std::fs::File::open(path)?;
        let meta = file.metadata()?;
        let len = meta.len() as usize;
        if len == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "File is empty",
            ));
        }

        extern "C" {
            fn mmap(
                addr: *mut std::ffi::c_void,
                length: usize,
                prot: i32,
                flags: i32,
                fd: i32,
                offset: i64,
            ) -> *mut std::ffi::c_void;
        }
        const PROT_READ: i32 = 1;
        const MAP_PRIVATE: i32 = 2;
        let map_failed = !0usize as *mut std::ffi::c_void;

        let fd = file.as_raw_fd();
        let ptr = unsafe {
            mmap(
                std::ptr::null_mut(),
                len,
                PROT_READ,
                MAP_PRIVATE,
                fd,
                0,
            )
        };
        if ptr == map_failed {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self { ptr, len })
    }

    fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr as *const u8, self.len) }
    }
}

#[cfg(unix)]
impl Drop for MmapFile {
    fn drop(&mut self) {
        let map_failed = !0usize as *mut std::ffi::c_void;
        if !self.ptr.is_null() && self.ptr != map_failed {
            extern "C" {
                fn munmap(addr: *mut std::ffi::c_void, length: usize) -> i32;
            }
            unsafe {
                munmap(self.ptr, self.len);
            }
        }
    }
}

enum ProgramBacking {
    Heap(Vec<u8>),
    #[cfg(unix)]
    Mmap(MmapFile),
}

impl std::ops::Deref for ProgramBacking {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            ProgramBacking::Heap(v) => v.as_slice(),
            #[cfg(unix)]
            ProgramBacking::Mmap(m) => m.as_slice(),
        }
    }
}

impl<T: AsRef<[u8]>> PartialEq<T> for ProgramBacking {
    fn eq(&self, other: &T) -> bool {
        self.as_slice() == other.as_ref()
    }
}

impl ProgramBacking {
    fn as_slice(&self) -> &[u8] {
        self
    }
}

struct Program {
    expected: usize,
    bytes: ProgramBacking,
    session: Option<Session>,
    last: String,
    source_dir: Option<std::path::PathBuf>,
}
thread_local! { static PROGRAM: RefCell<Option<Program>> = const { RefCell::new(None) }; }

fn discover_keys(source_dir: Option<&std::path::Path>) -> emulator_crypto::keys::KeyFile {
    let mut keys = emulator_crypto::keys::KeyFile::default();
    let mut prod_paths = Vec::new();
    let mut title_paths = Vec::new();

    if let Some(dir) = source_dir {
        prod_paths.push(dir.join("keys/prod.keys"));
        prod_paths.push(dir.join("prod.keys"));
        title_paths.push(dir.join("keys/title.keys"));
        title_paths.push(dir.join("title.keys"));
        if let Some(parent) = dir.parent() {
            prod_paths.push(parent.join("keys/prod.keys"));
            prod_paths.push(parent.join("prod.keys"));
            title_paths.push(parent.join("keys/title.keys"));
            title_paths.push(parent.join("title.keys"));
        }
    }

    for p in [
        "/data/user/0/com.algosculptor.emulator.gpulab/files/keys/prod.keys",
        "/data/user/0/com.algosculptor.emulator.gpulab/files/prod.keys",
        "/data/data/com.algosculptor.emulator.gpulab/files/keys/prod.keys",
        "/data/data/com.algosculptor.emulator.gpulab/files/prod.keys",
        "/sdcard/switch/prod.keys",
        "/storage/emulated/0/switch/prod.keys",
        "/sdcard/Download/prod.keys",
        "/storage/emulated/0/Download/prod.keys",
        "/sdcard/Documents/prod.keys",
        "/storage/emulated/0/Documents/prod.keys",
        "/sdcard/Android/data/org.yuzu.yuzu_emu/files/keys/prod.keys",
        "/storage/emulated/0/Android/data/org.yuzu.yuzu_emu/files/keys/prod.keys",
        "/sdcard/Android/data/dev.lemon.lemon_emu/files/keys/prod.keys",
        "/storage/emulated/0/Android/data/dev.lemon.lemon_emu/files/keys/prod.keys",
        "/sdcard/Android/data/org.suyu.suyu_emu/files/keys/prod.keys",
        "/storage/emulated/0/Android/data/org.suyu.suyu_emu/files/keys/prod.keys",
        "/sdcard/Android/data/org.sudachi.sudachi_emu/files/keys/prod.keys",
        "/storage/emulated/0/Android/data/org.sudachi.sudachi_emu/files/keys/prod.keys",
        "prod.keys",
        "keys.dat",
        "/root/.switch/prod.keys",
        "/data/local/tmp/prod.keys",
    ] {
        prod_paths.push(std::path::PathBuf::from(p));
    }

    for p in [
        "/data/user/0/com.algosculptor.emulator.gpulab/files/keys/title.keys",
        "/data/user/0/com.algosculptor.emulator.gpulab/files/title.keys",
        "/data/data/com.algosculptor.emulator.gpulab/files/keys/title.keys",
        "/data/data/com.algosculptor.emulator.gpulab/files/title.keys",
        "/sdcard/switch/title.keys",
        "/storage/emulated/0/switch/title.keys",
        "/sdcard/Download/title.keys",
        "/storage/emulated/0/Download/title.keys",
        "/sdcard/Documents/title.keys",
        "/storage/emulated/0/Documents/title.keys",
        "/sdcard/Android/data/org.yuzu.yuzu_emu/files/keys/title.keys",
        "/storage/emulated/0/Android/data/org.yuzu.yuzu_emu/files/keys/title.keys",
        "/sdcard/Android/data/dev.lemon.lemon_emu/files/keys/title.keys",
        "/storage/emulated/0/Android/data/dev.lemon.lemon_emu/files/keys/title.keys",
        "/sdcard/Android/data/org.suyu.suyu_emu/files/keys/title.keys",
        "/storage/emulated/0/Android/data/org.suyu.suyu_emu/files/keys/title.keys",
        "/sdcard/Android/data/org.sudachi.sudachi_emu/files/keys/title.keys",
        "/storage/emulated/0/Android/data/org.sudachi.sudachi_emu/files/keys/title.keys",
        "title.keys",
        "/root/.switch/title.keys",
        "/data/local/tmp/title.keys",
    ] {
        title_paths.push(std::path::PathBuf::from(p));
    }

    for pth in &prod_paths {
        if let Ok(data) = std::fs::read(pth) {
            let kf = emulator_crypto::keys::KeyFile::parse_lenient(&data);
            if kf.entry_count() > 0 {
                keys.merge(&kf);
            }
        }
    }

    for pth in &title_paths {
        if let Ok(data) = std::fs::read(pth) {
            let kf = emulator_crypto::keys::KeyFile::parse_lenient(&data);
            if kf.entry_count() > 0 {
                keys.merge(&kf);
            }
        }
    }

    keys
}
fn error(message: &str) -> String {
    format!(
        "{{\"ok\":false,\"state\":\"error\",\"error\":{}}}",
        quote(&message.chars().take(512).collect::<String>())
    )
}
fn report(f: impl FnOnce() -> Result<String, String>) -> EngineReport {
    let text = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err("Program boundary panic".into()))
        .unwrap_or_else(|e| error(&e));
    let text = if text.len() > REPORT_CAPACITY {
        error("Program response limit exceeded")
    } else {
        text
    };
    let mut out = EngineReport {
        abi_version: 2,
        length: text.len() as u32,
        bytes: [0; REPORT_CAPACITY],
    };
    out.bytes[..text.len()].copy_from_slice(text.as_bytes());
    out
}
fn access(f: impl FnOnce(&mut Program) -> Result<String, String>) -> EngineReport {
    report(|| {
        PROGRAM.with(|cell| {
            let mut state = cell
                .try_borrow_mut()
                .map_err(|_| "Program boundary busy".to_string())?;
            f(state
                .as_mut()
                .ok_or_else(|| "No program on this thread".to_string())?)
        })
    })
}
#[no_mangle]
pub extern "C" fn emulator_program_begin_v1(length: u32) -> u32 {
    std::panic::catch_unwind(|| {
        PROGRAM.with(|cell| {
            let Ok(mut state) = cell.try_borrow_mut() else {
                return 1;
            };
            if state.is_some() || !(128..=MAX_FILE).contains(&(length as usize)) {
                return 1;
            }
            let mut bytes = Vec::new();
            if bytes.try_reserve_exact(length as usize).is_err() {
                return 1;
            }
            *state = Some(Program {
                expected: length as usize,
                bytes: ProgramBacking::Heap(bytes),
                session: None,
                last: String::new(),
                source_dir: None,
            });
            0
        })
    })
    .unwrap_or(1)
}
#[no_mangle]
pub extern "C" fn emulator_program_append_v1(chunk: ProgramChunk) -> u32 {
    std::panic::catch_unwind(|| {
        PROGRAM.with(|cell| {
            let Ok(mut state) = cell.try_borrow_mut() else {
                return 1;
            };
            let Some(p) = state.as_mut() else {
                return 1;
            };
            let n = chunk.length as usize;
            if n == 0
                || n > 4096
                || p.session.is_some()
                || !p.last.is_empty()
                || n > p.expected.saturating_sub(p.bytes.len())
            {
                return 1;
            }
            if let ProgramBacking::Heap(ref mut b) = p.bytes {
                b.extend_from_slice(&chunk.bytes[..n]);
                0
            } else {
                1
            }
        })
    })
    .unwrap_or(1)
}

#[no_mangle]
pub extern "C" fn emulator_program_open_file_v1(path: *const std::os::raw::c_char) -> u32 {
    std::panic::catch_unwind(|| {
        if path.is_null() {
            return 1;
        }
        let c_str = unsafe { std::ffi::CStr::from_ptr(path) };
        let Ok(path_str) = c_str.to_str() else {
            return 1;
        };
        let file_path = std::path::Path::new(path_str);
        let Ok(metadata) = std::fs::metadata(file_path) else {
            return 1;
        };
        let file_len = metadata.len();
        const MAX_PATH_FILE: u64 = 64 * 1024 * 1024 * 1024; // 64 GiB
        if file_len < 128 || file_len > MAX_PATH_FILE {
            return 1;
        }
        PROGRAM.with(|cell| {
            let Ok(mut state) = cell.try_borrow_mut() else {
                return 1;
            };
            if state.is_some() {
                return 1;
            }
            let src_dir = file_path.parent().map(|p| p.to_path_buf());
            #[cfg(unix)]
            {
                if let Ok(mmap) = MmapFile::open(file_path) {
                    *state = Some(Program {
                        expected: file_len as usize,
                        bytes: ProgramBacking::Mmap(mmap),
                        session: None,
                        last: String::new(),
                        source_dir: src_dir.clone(),
                    });
                    return 0;
                }
            }
            if file_len <= (64 * 1024 * 1024) {
                if let Ok(bytes) = std::fs::read(file_path) {
                    *state = Some(Program {
                        expected: file_len as usize,
                        bytes: ProgramBacking::Heap(bytes),
                        session: None,
                        last: String::new(),
                        source_dir: src_dir,
                    });
                    return 0;
                }
            }
            1
        })
    })
    .unwrap_or(1)
}
#[no_mangle]
pub extern "C" fn emulator_program_inspect_v1() -> EngineReport {
    access(|p| {
        if p.bytes.len() != p.expected || p.session.is_some() || !p.last.is_empty() {
            return Err("Incomplete or already-started program".into());
        }
        if p.bytes.starts_with(b"PFS0") || p.bytes.starts_with(b"HFS0") {
            let _nsp = emulator_loader::nsp::NspArchive::parse(&p.bytes).map_err(|e| e.to_string())?;
            return Ok(format!(
                "{{\"ok\":true,\"state\":\"ready\",\"format\":\"NSP\",\"fileBytes\":{},\"memoryBytes\":{},\"mode\":\"package\",\"error\":\"\"}}",
                p.expected, p.expected
            ));
        }
        let image = NroImage::parse(&p.bytes).map_err(|e| e.to_string())?;
        let memory: usize = image.segments().iter().map(|s| s.memory_size).sum();
        Ok(format!("{{\"ok\":true,\"state\":\"ready\",\"format\":\"NRO0\",\"fileBytes\":{},\"memoryBytes\":{},\"mode\":{},\"error\":\"\"}}",
            p.expected,memory,quote(if p.bytes==CATCH {"catch"} else {"static"})))
    })
}
#[no_mangle]
pub extern "C" fn emulator_program_start_v1() -> EngineReport {
    access(|p| {
        if p.session.is_some() || !p.last.is_empty() || p.bytes.len() != p.expected {
            return Err("Incomplete or already-started program".into());
        }
        if p.bytes.starts_with(b"PFS0") || p.bytes.starts_with(b"HFS0") {
            let mut keys = discover_keys(p.source_dir.as_deref());
            if keys.header_key().is_none() {
                return Err("Nintendo Switch decryption keys (prod.keys) are required to decrypt and run NSP packages. Please install prod.keys in Settings -> Decryption Keys.".into());
            }
            p.session = Some(Session::from_nsp(&p.bytes, &mut keys).map_err(|e| e.to_string())?);
        } else {
            p.session = Some(Session::from_nro(&p.bytes).map_err(|e| e.to_string())?);
        }
        p.bytes = ProgramBacking::Heap(Vec::new());
        Ok(
            "{\"ok\":true,\"state\":\"running\",\"instructions\":0,\"output\":\"\",\"error\":\"\"}"
                .into(),
        )
    })
}
#[no_mangle]
pub extern "C" fn emulator_program_step_v1(budget: u32) -> EngineReport {
    access(|p| {
        if !(1..=4096).contains(&budget) {
            return Err("Invalid execution slice".into());
        }
        if !p.last.is_empty() {
            return Ok(p.last.clone());
        }
        let s = p
            .session
            .as_mut()
            .ok_or_else(|| "Program has not started".to_string())?;
        let remaining = MAX_INSTRUCTIONS.saturating_sub(s.instructions());
        let outcome = s.run(u64::from(budget).min(remaining));
        let (state, message) = match outcome {
            Ok(RunOutcome::Exited) => ("exited", String::new()),
            Ok(RunOutcome::Blocked) => ("waiting", String::new()),
            Ok(RunOutcome::BudgetExhausted) if s.instructions() >= MAX_INSTRUCTIONS => {
                ("limit", "Execution limit reached".into())
            }
            Ok(RunOutcome::BudgetExhausted) => ("running", String::new()),
            Err(e) => ("faulted", e.to_string()),
        };
        let total: usize = s.debug_records().iter().map(Vec::len).sum();
        let bytes: Vec<u8> = s
            .debug_records()
            .iter()
            .flat_map(|v| v.iter().copied())
            .take(2048)
            .collect();
        let decoded = String::from_utf8_lossy(&bytes);
        let truncated = total > bytes.len() || decoded.chars().count() > 512;
        let output = decoded.chars().take(512).collect::<String>();
        let text=format!("{{\"ok\":{},\"state\":{},\"instructions\":{},\"output\":{},\"outputTruncated\":{},\"error\":{}}}",
            message.is_empty(),quote(state),s.instructions(),quote(&output),truncated,quote(&message.chars().take(512).collect::<String>()));
        s.clear_ipc_trace();
        if state != "running" {
            p.session = None;
            p.last = text.clone();
        }
        Ok(text)
    })
}
#[no_mangle]
pub extern "C" fn emulator_program_close_v1() -> u32 {
    std::panic::catch_unwind(|| {
        PROGRAM.with(|cell| {
            let Ok(mut state) = cell.try_borrow_mut() else {
                return 1;
            };
            *state = None;
            0
        })
    })
    .unwrap_or(1)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn text(r: EngineReport) -> String {
        assert_eq!(r.abi_version, 2);
        String::from_utf8(r.bytes[..r.length as usize].to_vec()).unwrap()
    }
    fn upload(bytes: &[u8]) {
        assert_eq!(emulator_program_close_v1(), 0);
        assert_eq!(emulator_program_begin_v1(bytes.len() as u32), 0);
        for bytes in bytes.chunks(4096) {
            let mut c = ProgramChunk {
                length: bytes.len() as u32,
                bytes: [0; 4096],
            };
            c.bytes[..bytes.len()].copy_from_slice(bytes);
            assert_eq!(emulator_program_append_v1(c), 0);
        }
    }
    #[test]
    fn owned_programs_execute_from_uploaded_bytes_with_real_output_and_exit() {
        for (bytes, expected) in [
            (
                include_bytes!("../../../examples/homebrew/hello.nro").as_slice(),
                "Hello from original Switch homebrew!",
            ),
            (
                include_bytes!("../../../examples/homebrew/ipc-clone.nro").as_slice(),
                "Guest IPC clone retained counter=43",
            ),
            (
                include_bytes!("../../../examples/homebrew/threads.nro").as_slice(),
                "Guest threads woke events",
            ),
            (
                include_bytes!("../../../examples/homebrew/nv-draw.nro").as_slice(),
                "Guest draws passed",
            ),
        ] {
            upload(bytes);
            assert!(text(emulator_program_inspect_v1()).contains("\"mode\":\"static\""));
            assert!(text(emulator_program_start_v1()).contains("\"ok\":true"));
            let mut result = String::new();
            for _ in 0..10 {
                result = text(emulator_program_step_v1(1024));
                if !result.contains("\"state\":\"running\"") {
                    break;
                }
            }
            assert!(result.contains("\"state\":\"exited\""), "{result}");
            assert!(result.contains(expected), "{result}");
            assert_eq!(emulator_program_close_v1(), 0);
        }
    }
    #[test]
    fn upload_bounds_partial_input_duplicate_begin_and_abort_do_not_publish() {
        assert_eq!(std::mem::size_of::<ProgramChunk>(), 4100);
        assert_eq!(std::mem::offset_of!(ProgramChunk, bytes), 4);
        emulator_program_close_v1();
        assert_eq!(emulator_program_begin_v1(127), 1);
        assert_eq!(emulator_program_begin_v1(MAX_FILE as u32 + 1), 1);
        assert_eq!(emulator_program_begin_v1(128), 0);
        assert_eq!(emulator_program_begin_v1(128), 1);
        assert_eq!(
            emulator_program_append_v1(ProgramChunk {
                length: 129,
                bytes: [0; 4096]
            }),
            1
        );
        assert!(text(emulator_program_start_v1()).contains("Incomplete"));
        assert_eq!(
            emulator_program_append_v1(ProgramChunk {
                length: 128,
                bytes: [0; 4096]
            }),
            0
        );
        assert!(text(emulator_program_inspect_v1()).contains("missing NRO0"));
        emulator_program_close_v1();
        assert_eq!(
            emulator_program_append_v1(ProgramChunk {
                length: 1,
                bytes: [0; 4096]
            }),
            1
        );
        upload(CATCH);
        assert!(text(emulator_program_inspect_v1()).contains("\"mode\":\"catch\""));
        emulator_program_close_v1();

        // NSP package inspection
        let mut pfs0 = vec![0u8; 128];
        pfs0[..4].copy_from_slice(b"PFS0");
        upload(&pfs0);
        let nsp_report = text(emulator_program_inspect_v1());
        assert!(nsp_report.contains("\"format\":\"NSP\""), "{nsp_report}");
        assert!(nsp_report.contains("\"mode\":\"package\""), "{nsp_report}");
        let start_report = text(emulator_program_start_v1());
        assert!(start_report.contains("prod.keys"), "{start_report}");
        emulator_program_close_v1();
    }
    #[test]
    fn faults_and_instruction_limit_are_explicit_and_release_the_session() {
        let mut bytes = include_bytes!("../../../examples/homebrew/hello.nro").to_vec();
        bytes[..4].copy_from_slice(&0u32.to_le_bytes());
        upload(&bytes);
        emulator_program_start_v1();
        assert!(text(emulator_program_step_v1(64)).contains("\"state\":\"faulted\""));
        assert!(PROGRAM.with(|p| p.borrow().as_ref().unwrap().session.is_none()));
        bytes[..4].copy_from_slice(&0x14000000u32.to_le_bytes());
        upload(&bytes);
        emulator_program_start_v1();
        for _ in 0..489 {
            let r = text(emulator_program_step_v1(4096));
            if r.contains("\"state\":\"limit\"") {
                assert!(r.contains("2000000"));
                assert!(PROGRAM.with(|p| p.borrow().as_ref().unwrap().session.is_none()));
                emulator_program_close_v1();
                return;
            }
        }
        panic!("Missing execution limit");
    }
    #[test]
    fn wrong_thread_and_invalid_slice_cannot_advance_or_replace_a_program() {
        upload(include_bytes!("../../../examples/homebrew/hello.nro"));
        emulator_program_start_v1();
        assert!(std::thread::spawn(|| text(emulator_program_step_v1(1)))
            .join()
            .unwrap()
            .contains("No program"));
        assert!(text(emulator_program_step_v1(0)).contains("Invalid execution slice"));
        assert!(text(emulator_program_step_v1(4097)).contains("Invalid execution slice"));
        assert!(text(emulator_program_step_v1(8)).contains("\"instructions\":5"));
        emulator_program_close_v1();
    }
    #[test]
    fn open_file_v1_loads_and_executes_program_from_disk() {
        emulator_program_close_v1();
        let path = std::ffi::CString::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/homebrew/hello.nro"
        ))
        .unwrap();
        assert_eq!(emulator_program_open_file_v1(path.as_ptr()), 0);
        let inspected = text(emulator_program_inspect_v1());
        assert!(inspected.contains("\"format\":\"NRO0\""), "{inspected}");
        assert!(inspected.contains("\"ok\":true"), "{inspected}");
        let started = text(emulator_program_start_v1());
        assert!(started.contains("\"ok\":true"), "{started}");
        let mut result = String::new();
        for _ in 0..10 {
            result = text(emulator_program_step_v1(1024));
            if !result.contains("\"state\":\"running\"") {
                break;
            }
        }
        assert!(result.contains("\"state\":\"exited\""), "{result}");
        assert!(result.contains("Hello from original Switch homebrew!"), "{result}");
        assert_eq!(emulator_program_close_v1(), 0);
    }
}
