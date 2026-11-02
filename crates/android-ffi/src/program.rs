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
struct Program {
    expected: usize,
    bytes: Vec<u8>,
    session: Option<Session>,
    last: String,
}
thread_local! { static PROGRAM: RefCell<Option<Program>> = const { RefCell::new(None) }; }
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
                bytes,
                session: None,
                last: String::new(),
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
            p.bytes.extend_from_slice(&chunk.bytes[..n]);
            0
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
        p.session = Some(Session::from_nro(&p.bytes).map_err(|e| e.to_string())?);
        p.bytes = Vec::new();
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
}
