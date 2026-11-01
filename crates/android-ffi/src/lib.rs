//! Fixed-size value-only C ABI for original guest reference workloads.
//! C callers receive initialized bytes, never Rust pointers or allocations.
use emulator_session::{RunOutcome, Session, LOAD_BASE};
use std::fmt::Write;
mod gpu_core;
mod graphics;
pub use graphics::ClearReplay;

pub const REPORT_CAPACITY: usize = 8192;
#[repr(C)]
pub struct EngineReport {
    pub abi_version: u32,
    pub length: u32,
    pub bytes: [u8; REPORT_CAPACITY],
}

fn quote(text: &str) -> String {
    let mut result = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c < ' ' => write!(result, "\\u{:04x}", c as u32).expect("String formatting"),
            c => result.push(c),
        }
    }
    result.push('"');
    result
}

struct Case {
    name: &'static str,
    image: &'static [u8],
    message: &'static str,
    instructions: u64,
    ipc_calls: usize,
    memory: Option<(u64, u64)>,
    scheduler: (usize, u64, u64), // threads, blocking waits, timeouts
    cancellations: u64,
    ipc_buffers: (usize, usize, usize),
    copy_handles: usize,
}
fn reference_checks() -> String {
    let cases = [
        Case {
            name: "static-nro-debug-exit",
            image: include_bytes!("../../../examples/homebrew/hello.nro"),
            message: "Hello from original Switch homebrew!\n",
            instructions: 5,
            ipc_calls: 0,
            memory: None,
            scheduler: (1, 0, 0),
            cancellations: 0,
            ipc_buffers: (0, 0, 0),
            copy_handles: 0,
        },
        Case {
            name: "guest-service-manager-ipc",
            image: include_bytes!("../../../examples/homebrew/ipc-counter.nro"),
            message: "Guest IPC counter reached 42; handles closed.\n",
            instructions: 384,
            ipc_calls: 5,
            memory: Some((0x2000, 42)),
            scheduler: (1, 0, 0),
            cancellations: 0,
            ipc_buffers: (0, 0, 0),
            copy_handles: 0,
        },
        Case {
            name: "guest-cloned-session-lifetime",
            image: include_bytes!("../../../examples/homebrew/ipc-clone.nro"),
            message: "Guest IPC clone retained counter=43 after close.\n",
            instructions: 529,
            ipc_calls: 7,
            memory: Some((0x2000, 43)),
            scheduler: (1, 0, 0),
            cancellations: 0,
            ipc_buffers: (0, 0, 0),
            copy_handles: 0,
        },
        Case {
            name: "guest-event-signaling-polling",
            image: include_bytes!("../../../examples/homebrew/events.nro"),
            message: "Guest events signaled, waited, reset and closed.\n",
            instructions: 87,
            ipc_calls: 0,
            memory: Some((0x2008, 7)),
            scheduler: (1, 0, 0),
            cancellations: 0,
            ipc_buffers: (0, 0, 0),
            copy_handles: 0,
        },
        Case {
            name: "guest-thread-scheduling-and-blocking-waits",
            image: include_bytes!("../../../examples/homebrew/threads.nro"),
            message: "Guest threads woke events, joined and timed out.\n",
            instructions: 97,
            ipc_calls: 0,
            memory: Some((0x2020, 42)),
            scheduler: (2, 3, 1),
            cancellations: 0,
            ipc_buffers: (0, 0, 0),
            copy_handles: 0,
        },
        Case {
            name: "guest-wait-cancellation",
            image: include_bytes!("../../../examples/homebrew/cancellation.nro"),
            message: "Guest cancellation interrupted waits exactly once.\n",
            instructions: 106,
            ipc_calls: 0,
            memory: Some((0x2020, 42)),
            scheduler: (2, 2, 0),
            cancellations: 2,
            ipc_buffers: (0, 0, 0),
            copy_handles: 0,
        },
        Case {
            name: "guest-gpu-dma-submission",
            image: include_bytes!("../../../examples/homebrew/gpu-dma.nro"),
            message: "Guest DMA copied 16 bytes; dependent copies verified.\n",
            instructions: 338,
            ipc_calls: 4,
            memory: Some((0x2030, 42)),
            scheduler: (1, 0, 0),
            cancellations: 0,
            ipc_buffers: (0, 0, 0),
            copy_handles: 0,
        },
        Case {
            name: "guest-hipc-buffered-dma",
            image: include_bytes!("../../../examples/homebrew/gpu-buffers.nro"),
            message: "Guest HIPC buffers submitted DMA and returned bytes.\n",
            instructions: 327,
            ipc_calls: 4,
            memory: Some((0x2020, 42)),
            scheduler: (1, 0, 0),
            cancellations: 0,
            ipc_buffers: (3, 1, 80),
            copy_handles: 0,
        },
        Case {
            name: "guest-nv-transfer-memory-lifetime",
            image: include_bytes!("../../../examples/homebrew/nv-initialize.nro"),
            message: "Guest NV initialized with copied handles; memory restored.\n",
            instructions: 317,
            ipc_calls: 4,
            memory: Some((0x2000, 42)),
            scheduler: (1, 0, 0),
            cancellations: 0,
            ipc_buffers: (0, 0, 0),
            copy_handles: 2,
        },
        Case {
            name: "guest-nvmap-device-ioctls",
            image: include_bytes!("../../../examples/homebrew/nvmap.nro"),
            message: "Guest NVMap created, queried and freed; descriptors closed.\n",
            instructions: 719,
            ipc_calls: 9,
            memory: Some((0x3030, 4096)),
            scheduler: (1, 0, 0),
            cancellations: 0,
            ipc_buffers: (4, 3, 98),
            copy_handles: 2,
        },
        Case {
            name: "guest-nvmap-backing-import",
            image: include_bytes!("../../../examples/homebrew/nvmap-alloc.nro"),
            message: "Guest NVMap imported backing, queried alignment and freed.\n",
            instructions: 817,
            ipc_calls: 10,
            memory: Some((0x4000, 77)),
            scheduler: (1, 0, 0),
            cancellations: 0,
            ipc_buffers: (5, 4, 162),
            copy_handles: 2,
        },
        Case {
            name: "guest-nv-gpu-address-space",
            image: include_bytes!("../../../examples/homebrew/nv-gpu-map.nro"),
            message: "Guest NV GPU mappings retained backing and unmapped cleanly.\n",
            instructions: 3172,
            ipc_calls: 15,
            memory: Some((0x3040, 1 << 32)),
            scheduler: (1, 0, 0),
            cancellations: 0,
            ipc_buffers: (9, 6, 372),
            copy_handles: 2,
        },
        Case {
            name: "guest-nv-channel-batch",
            image: include_bytes!("../../../examples/homebrew/nv-channel-batch.nro"),
            message: "Guest NV batch passed: jobs, fences, rejection and lifetime.\n",
            instructions: 7179,
            ipc_calls: 32,
            memory: Some((0x4040, 3)),
            scheduler: (1, 0, 0),
            cancellations: 0,
            ipc_buffers: (25, 18, 875),
            copy_handles: 2,
        },
        Case {
            name: "guest-nv-color-clears",
            image: graphics::IMAGE,
            message: graphics::MESSAGE,
            instructions: 5571,
            ipc_calls: 25,
            memory: Some((0x4040, 2)),
            scheduler: (1, 0, 0),
            cancellations: 0,
            ipc_buffers: (18, 11, 603),
            copy_handles: 2,
        },
    ];
    let mut all_passed = true;
    let mut tests = Vec::new();
    for case in cases {
        let mut instructions = 0;
        let mut ipc_calls = 0;
        let mut copies = 0;
        let mut buffers = (0, 0, 0);
        let mut handles = 0;
        let mut debug = String::new();
        let mut scheduling = (0, 0, 0, 0, 0, 0);
        let check = (|| -> Result<(), String> {
            let mut session = Session::from_nro(case.image).map_err(|e| e.to_string())?;
            let outcome = session.run(10000).map_err(|e| e.to_string())?;
            instructions = session.instructions();
            ipc_calls = session.kernel().ipc_records().len();
            for record in session.kernel().ipc_records() {
                copies += record.copy_handles;
                buffers.0 += record.send_buffers;
                buffers.1 += record.receive_buffers;
                buffers.2 += record.buffer_bytes;
            }
            handles = session.kernel().live_handle_count();
            let stats = session.kernel().scheduler_stats();
            scheduling = (
                stats.threads_created,
                stats.context_switches,
                stats.blocking_waits,
                stats.timeouts,
                stats.virtual_time_ns,
                stats.cancelled_waits,
            );
            for record in session.debug_records() {
                debug.push_str(&String::from_utf8_lossy(record));
            }
            if outcome != RunOutcome::Exited
                || instructions != case.instructions
                || ipc_calls != case.ipc_calls
                || buffers != case.ipc_buffers
                || copies != case.copy_handles
                || session.kernel().live_transfer_memory_count() != 0
                || session.memory().lent_bytes() != 0
                || handles != 0
                || session.debug_records() != [case.message.as_bytes().to_vec()]
                || (stats.threads_created, stats.blocking_waits, stats.timeouts) != case.scheduler
                || stats.cancelled_waits != case.cancellations
            {
                return Err("Reference guest output, lifetime or execution trace mismatch".into());
            }
            if case.name == "guest-nv-color-clears" {
                graphics::verify_guest(&session)?;
            }
            if let Some((offset, value)) = case.memory {
                if session
                    .memory()
                    .read_u64(LOAD_BASE + offset)
                    .map_err(|e| e.to_string())?
                    != value
                {
                    return Err("Reference guest memory result mismatch".into());
                }
            }
            if case.name == "guest-thread-scheduling-and-blocking-waits" {
                for (address, expected) in [
                    (LOAD_BASE + 0x2008, 42),
                    (LOAD_BASE + 0x2010, LOAD_BASE + 0x2000),
                    (LOAD_BASE + 0x2018, 0x7ffe1200),
                    (LOAD_BASE + 0x2028, LOAD_BASE + 0x2000),
                    (0x7ffe0280, 0x1111),
                    (0x7ffe1280, 0x2222),
                ] {
                    if session
                        .memory()
                        .read_u64(address)
                        .map_err(|e| e.to_string())?
                        != expected
                    {
                        return Err("Thread argument, TLS or wait-result mismatch".into());
                    }
                }
                if stats.context_switches < 4 || stats.virtual_time_ns < 1600 {
                    return Err("Guest scheduling or virtual-time progress mismatch".into());
                }
            }
            if case.cancellations > 0 {
                for offset in [0x2018, 0x2028] {
                    if session
                        .memory()
                        .read_u64(LOAD_BASE + offset)
                        .map_err(|e| e.to_string())?
                        != LOAD_BASE + 0x2000
                    {
                        return Err("Cancellation changed the wait's output index".into());
                    }
                }
            }
            Ok(())
        })();
        let passed = check.is_ok();
        all_passed &= passed;
        tests.push(format!("{{\"name\":{},\"passed\":{},\"instructions\":{},\"ipcCalls\":{},\"copyHandles\":{},\"sendBuffers\":{},\"receiveBuffers\":{},\"bufferBytes\":{},\"handlesAfterExit\":{},\"threadsCreated\":{},\"contextSwitches\":{},\"blockingWaits\":{},\"timeouts\":{},\"virtualTimeNs\":{},\"cancelledWaits\":{},\"debugOutput\":{},\"error\":{}}}",
            quote(case.name),passed,instructions,ipc_calls,copies,buffers.0,buffers.1,buffers.2,handles,scheduling.0,scheduling.1,scheduling.2,scheduling.3,scheduling.4,scheduling.5,quote(&debug),quote(&check.err().unwrap_or_default())));
    }
    format!("{{\"scope\":\"original-guest-reference-programs\",\"suiteVersion\":11,\"cpuBackend\":\"original-interpreter\",\"hostArchitecture\":{},\"guestArchitecture\":\"aarch64\",\"tests\":[{}],\"allPassed\":{}}}",
        quote(std::env::consts::ARCH),tests.join(","),all_passed)
}

/// C ABI layout is mirrored/checked in jni.cpp.
/// Panics never unwind through C; output has no pointers and needs no free call.
#[no_mangle]
pub extern "C" fn emulator_run_reference_guests_v2() -> EngineReport {
    encode_report(reference_checks, "original-guest-reference-programs")
}

#[no_mangle]
pub extern "C" fn emulator_run_gpu_core_checks_v2() -> EngineReport {
    encode_report(
        gpu_core::report,
        "original-gpu-memory-and-command-reference",
    )
}

fn encode_report(run: fn() -> String, scope: &str) -> EngineReport {
    let mut json = std::panic::catch_unwind(run).unwrap_or_else(|_| {
        format!(
            "{{\"scope\":{},\"allPassed\":false,\"error\":\"Engine panic\"}}",
            quote(scope)
        )
    });
    if json.len() > REPORT_CAPACITY {
        json = format!(
            "{{\"scope\":{},\"allPassed\":false,\"error\":\"Report capacity exceeded\"}}",
            quote(scope)
        );
    }
    let mut report = EngineReport {
        abi_version: 2,
        length: json.len() as u32,
        bytes: [0; REPORT_CAPACITY],
    };
    report.bytes[..json.len()].copy_from_slice(json.as_bytes());
    report
}

/// Integrated NV protocol regressions, distinct from interpreted guest programs.
#[no_mangle]
pub extern "C" fn emulator_run_nv_checks_v2() -> EngineReport {
    encode_report(nv_reference_checks, "original-nv-protocol-reference")
}
fn nv_reference_checks() -> String {
    let cases = emulator_services::reference::run();
    let passed = cases.iter().all(|c| c.passed);
    let tests=cases.iter().map(|c|format!("{{\"name\":{},\"passed\":{},\"ipcCalls\":{},\"verifiedBytes\":{},\"handlesAfterExit\":{},\"error\":{}}}",quote(c.name),c.passed,c.ipc_calls,c.verified_bytes,c.handles_after_exit.map_or_else(||"null".into(),|v|v.to_string()),quote(&c.error))).collect::<Vec<_>>().join(",");
    format!("{{\"scope\":\"original-nv-protocol-reference\",\"suiteVersion\":2,\"executesGuestCpu\":false,\"hostArchitecture\":{},\"tests\":[{}],\"allPassed\":{}}}",quote(std::env::consts::ARCH),tests,passed)
}

/// Fixed-size owned clear-plan/image bridge; no Rust pointers cross the ABI.
#[no_mangle]
pub extern "C" fn emulator_guest_clear_replay_v1(index: u32) -> ClearReplay {
    graphics::replay(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn c_abi_returns_bounded_initialized_reference_results() {
        assert_eq!(std::mem::size_of::<EngineReport>(), 8200);
        assert_eq!(std::mem::align_of::<EngineReport>(), 4);
        let result = emulator_run_reference_guests_v2();
        assert_eq!(result.abi_version, 2);
        let text = std::str::from_utf8(&result.bytes[..result.length as usize]).unwrap();
        assert!(text.ends_with("\"allPassed\":true}"));
        assert_eq!(text.matches("\"passed\":true").count(), 14);
        assert!(result.bytes[result.length as usize..]
            .iter()
            .all(|b| *b == 0));
    }
    #[test]
    fn gpu_core_checks_use_actual_shared_memory_and_retire_all_resources() {
        let report = emulator_run_gpu_core_checks_v2();
        assert_eq!(report.abi_version, 2);
        let text = std::str::from_utf8(&report.bytes[..report.length as usize]).unwrap();
        assert!(text.ends_with("\"allPassed\":true}"));
        assert_eq!(text.matches("\"passed\":true").count(), 3);
        assert!(text.contains(
            "\"capturedSubmissions\":2,\"decodedWrites\":4,\"residentBytesAfterRelease\":0"
        ));
        assert!(text.contains("\"copyLaunches\":2,\"copiedBytes\":16"));
        assert!(text.contains("maxwell-dma-linear-copy"));
        assert!(report.bytes[report.length as usize..]
            .iter()
            .all(|b| *b == 0));
    }

    #[test]
    fn nv_batch_reports_actual_protocol_results_and_bounded_bytes() {
        let result = super::emulator_run_nv_checks_v2();
        let text = std::str::from_utf8(&result.bytes[..result.length as usize]).unwrap();
        assert_eq!(result.abi_version, 2);
        assert!(text.ends_with("\"allPassed\":true}"));
        assert_eq!(text.matches("\"passed\":true").count(), 34);
        assert_eq!(text.matches("\"handlesAfterExit\":0").count(), 34);
        assert!(result.bytes[result.length as usize..]
            .iter()
            .all(|b| *b == 0));
    }
}
