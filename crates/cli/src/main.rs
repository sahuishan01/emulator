use emulator_a64::{Cpu, StepEvent};
use emulator_loader::MAX_NRO_FILE_SIZE;
use emulator_memory::{AddressSpace, Permissions};
use emulator_session::{RunOutcome, Session};
use std::{env, fs::File, io::Read, process::ExitCode};

const CODE_BASE: u64 = 0x10_0000;
const STACK_BASE: u64 = 0x8000_0000;
const STACK_SIZE: usize = 0x1_0000;
const MAX_IMAGE: usize = 16 * 1024 * 1024;
const DEFAULT_STEPS: u64 = 100_000;
const NSP_MAX_STEPS: u64 = 1_000_000_000_000;
const NSP_SAMPLE_INTERVAL: u64 = 4096;
const DEMO: &[u8] = include_bytes!("../../../examples/a64/demo.bin");

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments
        .first()
        .is_some_and(|argument| argument == "run-nro")
    {
        return run_nro(&arguments);
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == "run-nca")
    {
        return run_nca(&arguments);
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == "run-nsp")
    {
        return run_nsp(&arguments);
    }
    let (program, max_steps, demo) = match arguments.as_slice() {
        [command] if command == "demo" => (DEMO.to_vec(), DEFAULT_STEPS, true),
        [command, file] if command == "run-raw" => (read_image(file)?, DEFAULT_STEPS, false),
        [command, file, flag, limit] if command == "run-raw" && flag == "--max-steps" => {
            let max_steps: u64 = limit.parse()?;
            if max_steps == 0 || max_steps > 10_000_000 {
                return Err("step limit must be between 1 and 10000000".into());
            }
            (read_image(file)?, max_steps, false)
        }
        _ => return Err("usage: emulator-cli demo | run-raw FILE [--max-steps N] | run-nro FILE [--max-steps N] | run-nca FILE [--keys KEYFILE] [--titlekeys TITLEKEYS] [--max-steps N] | run-nsp FILE [--keys KEYFILE] [--titlekeys TITLEKEYS] [--max-steps N]".into()),
    };
    let mut memory = AddressSpace::default();
    memory.map(CODE_BASE, program, Permissions::READ_EXECUTE)?;
    memory.map(0x2000, vec![0; 4096], Permissions::READ_WRITE)?;
    memory.map(STACK_BASE, vec![0; STACK_SIZE], Permissions::READ_WRITE)?;
    let mut cpu = Cpu::new(CODE_BASE, STACK_BASE + STACK_SIZE as u64);
    for step in 1..=max_steps {
        if let StepEvent::SupervisorCall { immediate } = cpu.step(&mut memory)? {
            println!("Stopped at SVC #{immediate:#x} after {step} instructions; no OS service was executed.");
            println!(
                "x0={} x1={} x2={:#x} x3={} pc={:#x}",
                cpu.x[0], cpu.x[1], cpu.x[2], cpu.x[3], cpu.pc
            );
            if demo {
                let stored = memory.read_u64(0x2000)?;
                if immediate != 0x42 || cpu.x[0] != 35 || cpu.x[3] != 42 || stored != 35 {
                    return Err(
                        "demo result did not match its independently assembled program".into(),
                    );
                }
                println!("Demo passed: loop sum=35, guest memory=35, subroutine result=42.");
            }
            return Ok(());
        }
    }
    Err(format!(
        "step budget exhausted at pc={:#x}; no supervisor call reached",
        cpu.pc
    )
    .into())
}

fn run_nro(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let (file, budget) = match arguments {
        [_, file] => (file, DEFAULT_STEPS),
        [_, file, flag, budget] if flag == "--max-steps" => (file, budget.parse::<u64>()?),
        _ => return Err("usage: emulator-cli run-nro FILE [--max-steps N]".into()),
    };
    if budget == 0 || budget > 10_000_000 {
        return Err("step limit must be between 1 and 10000000".into());
    }
    let mut bytes = Vec::new();
    File::open(file)?
        .take((MAX_NRO_FILE_SIZE + 1) as u64)
        .read_to_end(&mut bytes)?;
    let mut session = Session::from_nro(&bytes)?;
    let outcome = session.run(budget);
    for record in session.debug_records() {
        let text = String::from_utf8_lossy(record);
        for character in text.chars() {
            if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
                print!("{}", character.escape_default());
            } else {
                print!("{character}");
            }
        }
    }
    match outcome? {
        RunOutcome::Exited => {
            println!(
                "Guest process exited after {} instructions.",
                session.instructions()
            );
            Ok(())
        }
        RunOutcome::BudgetExhausted => {
            Err(format!("NRO step budget exhausted at pc={:#x}", session.cpu().pc).into())
        }
        RunOutcome::Blocked => {
            Err("guest process is blocked with no runnable thread or scheduled timeout".into())
        }
    }
}

/// Real console dumps carry RSA blobs and other entries that are not 16/32-byte
/// AES keys, so explicit files use the lenient parser (as the Android path does).
/// A file that yields no usable key at all is still an explicit error.
fn read_key_file(path: &str) -> Result<emulator_crypto::keys::KeyFile, Box<dyn std::error::Error>> {
    let content = std::fs::read(path)?;
    let keys = emulator_crypto::keys::KeyFile::parse_lenient(&content);
    if keys.entry_count() == 0 {
        return Err(format!("{path}: no usable 16- or 32-byte `name = hex` entries").into());
    }
    Ok(keys)
}

fn load_keys(
    keys_path: Option<&str>,
    titlekeys_path: Option<&str>,
) -> Result<emulator_crypto::keys::KeyFile, Box<dyn std::error::Error>> {
    let mut keys = if let Some(path) = keys_path {
        read_key_file(path)?
    } else {
        let default_paths = ["prod.keys", "keys.dat", "/root/.switch/prod.keys"];
        let mut loaded = None;
        for p in default_paths {
            if let Ok(content) = std::fs::read(p) {
                if let Ok(kf) = emulator_crypto::keys::KeyFile::parse(&content) {
                    loaded = Some(kf);
                    break;
                }
            }
        }
        loaded.unwrap_or_default()
    };

    if let Some(path) = titlekeys_path {
        let title_keys = read_key_file(path)?;
        keys.merge(&title_keys);
    } else {
        let default_title_paths = ["title.keys", "/root/.switch/title.keys"];
        for p in default_title_paths {
            if let Ok(content) = std::fs::read(p) {
                if let Ok(tk) = emulator_crypto::keys::KeyFile::parse(&content) {
                    keys.merge(&tk);
                    break;
                }
            }
        }
    }

    Ok(keys)
}

fn run_nca(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut file: Option<&str> = None;
    let mut keys_path: Option<&str> = None;
    let mut titlekeys_path: Option<&str> = None;
    let mut budget = DEFAULT_STEPS;

    let mut i = 1;
    while i < arguments.len() {
        match arguments[i].as_str() {
            "--keys" => {
                i += 1;
                if i >= arguments.len() {
                    return Err("--keys requires a file path".into());
                }
                keys_path = Some(&arguments[i]);
            }
            "--titlekeys" => {
                i += 1;
                if i >= arguments.len() {
                    return Err("--titlekeys requires a file path".into());
                }
                titlekeys_path = Some(&arguments[i]);
            }
            "--max-steps" => {
                i += 1;
                if i >= arguments.len() {
                    return Err("--max-steps requires an integer argument".into());
                }
                budget = arguments[i].parse::<u64>()?;
            }
            arg if !arg.starts_with("--") && file.is_none() => {
                file = Some(arg);
            }
            arg => return Err(format!("unexpected argument: {arg}").into()),
        }
        i += 1;
    }

    let file = file.ok_or("usage: emulator-cli run-nca FILE [--keys KEYFILE] [--titlekeys TITLEKEYS] [--max-steps N] [--disasm HEXADDR:COUNT]... [--peek HEXADDR:COUNT]...")?;
    if budget == 0 || budget > 10_000_000 {
        return Err("step limit must be between 1 and 10000000".into());
    }

    let keys = load_keys(keys_path, titlekeys_path)?;

    let mut nca_bytes = Vec::new();
    File::open(file)?.read_to_end(&mut nca_bytes)?;

    println!("Loading NCA3 archive: {file} ({} bytes)", nca_bytes.len());
    let mut session = Session::from_nca(&nca_bytes, &keys)?;
    println!(
        "Program successfully mapped into guest address space. Initial PC={:#x}, SP={:#x}",
        session.cpu().pc,
        session.cpu().sp
    );

    let outcome = session.run(budget);
    for record in session.debug_records() {
        let text = String::from_utf8_lossy(record);
        for character in text.chars() {
            if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
                print!("{}", character.escape_default());
            } else {
                print!("{character}");
            }
        }
    }

    match outcome? {
        RunOutcome::Exited => {
            println!(
                "Guest process exited successfully after {} instructions.",
                session.instructions()
            );
            Ok(())
        }
        RunOutcome::BudgetExhausted => {
            println!(
                "NCA prototype reached instruction step budget ({}) at PC={:#x}",
                session.instructions(),
                session.cpu().pc
            );
            Ok(())
        }
        RunOutcome::Blocked => Err("guest process is blocked waiting for kernel resources".into()),
    }
}

fn run_nsp(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut file: Option<&str> = None;
    let mut keys_path: Option<&str> = None;
    let mut titlekeys_path: Option<&str> = None;
    let mut budget = DEFAULT_STEPS;
    let mut disasm: Vec<(u64, u64)> = Vec::new();
    let mut peek: Vec<(u64, u64)> = Vec::new();
    let mut watch: Option<(u64, u64)> = None;

    let mut i = 1;
    while i < arguments.len() {
        match arguments[i].as_str() {
            "--keys" => {
                i += 1;
                if i >= arguments.len() {
                    return Err("--keys requires a file path".into());
                }
                keys_path = Some(&arguments[i]);
            }
            "--titlekeys" => {
                i += 1;
                if i >= arguments.len() {
                    return Err("--titlekeys requires a file path".into());
                }
                titlekeys_path = Some(&arguments[i]);
            }
            "--max-steps" => {
                i += 1;
                if i >= arguments.len() {
                    return Err("--max-steps requires an integer argument".into());
                }
                budget = arguments[i].parse::<u64>()?;
            }
            "--disasm" => {
                i += 1;
                let spec = arguments.get(i).ok_or("--disasm requires ADDR:COUNT")?;
                let (address, count) = spec.split_once(':').ok_or("--disasm expects ADDR:COUNT")?;
                let address = u64::from_str_radix(address.trim_start_matches("0x"), 16)?;
                let count = count.parse::<u64>()?;
                if count == 0 || count > 4096 {
                    return Err("--disasm COUNT must be between 1 and 4096".into());
                }
                disasm.push((address, count));
            }
            "--peek" => {
                i += 1;
                let spec = arguments.get(i).ok_or("--peek requires ADDR:COUNT")?;
                let (address, count) = spec.split_once(':').ok_or("--peek expects ADDR:COUNT")?;
                let address = u64::from_str_radix(address.trim_start_matches("0x"), 16)?;
                let count = count.parse::<u64>()?;
                if count == 0 || count > 4096 {
                    return Err("--peek COUNT must be between 1 and 4096".into());
                }
                peek.push((address, count));
            }
            "--watch" => {
                i += 1;
                let spec = arguments.get(i).ok_or("--watch requires HEXADDR:LEN")?;
                let (address, len) = spec.split_once(':').ok_or("--watch expects HEXADDR:LEN")?;
                let address = u64::from_str_radix(address.trim_start_matches("0x"), 16)?;
                let len = len.parse::<u64>()?;
                if len == 0 || len > 4096 {
                    return Err("--watch LEN must be between 1 and 4096".into());
                }
                watch = Some((address, len));
            }
            arg if !arg.starts_with("--") && file.is_none() => {
                file = Some(arg);
            }
            arg => return Err(format!("unexpected argument: {arg}").into()),
        }
        i += 1;
    }

    let file = file.ok_or("usage: emulator-cli run-nsp FILE [--keys KEYFILE] [--titlekeys TITLEKEYS] [--max-steps N] [--disasm HEXADDR:COUNT]... [--peek HEXADDR:COUNT]... [--watch HEXADDR:LEN]")?;
    // Commercial titles need far more than the microprogram cap; the caller bounds
    // wall-clock time (the smoke sandbox sets a runtime limit).
    if budget == 0 || budget > NSP_MAX_STEPS {
        return Err(format!("step limit must be between 1 and {NSP_MAX_STEPS}").into());
    }

    let mut keys = load_keys(keys_path, titlekeys_path)?;

    let mut nsp_bytes = Vec::new();
    File::open(file)?.read_to_end(&mut nsp_bytes)?;

    println!("Loading NSP package: {file} ({} bytes)", nsp_bytes.len());
    let nsp = emulator_loader::nsp::NspArchive::parse(&nsp_bytes)?;
    println!("NSP contains {} file entries:", nsp.filenames().len());
    for name in nsp.filenames() {
        println!("  - {name}");
    }

    let tickets_imported = nsp.import_tickets(&mut keys);
    if tickets_imported > 0 {
        println!("Imported {tickets_imported} title key(s) from NSP ticket(s).");
    }

    if let Ok(Some(cnmt)) = nsp.parse_cnmt(&keys) {
        println!(
            "Parsed CNMT metadata: Title ID {:#018x}, Version {:#x}, {} content records",
            cnmt.title_id,
            cnmt.version,
            cnmt.contents.len()
        );
        for content in &cnmt.contents {
            println!(
                "  Content {:?}: {} ({} bytes)",
                content.content_type,
                content.nca_filename(),
                content.size
            );
        }
    }

    let mut session = Session::from_nsp_owned(std::rc::Rc::new(nsp_bytes), &mut keys)?;
    println!(
        "Program successfully mapped into guest address space. Initial PC={:#x}, SP={:#x}",
        session.cpu().pc,
        session.cpu().sp
    );
    if let Some((address, len)) = watch {
        session.set_write_watch(address, len);
    }

    // Run in slices and sample the PC at each boundary: a cheap profile that
    // separates real progress from a spin loop when the budget runs out.
    let started = std::time::Instant::now();
    let mut samples: std::collections::BTreeMap<u64, u64> = std::collections::BTreeMap::new();
    let mut remaining = budget;
    let outcome = loop {
        let slice = remaining.min(NSP_SAMPLE_INTERVAL);
        match session.run(slice) {
            Ok(RunOutcome::BudgetExhausted) => {
                remaining -= slice;
                *samples.entry(session.cpu().pc).or_insert(0) += 1;
                if remaining == 0 {
                    break Ok(RunOutcome::BudgetExhausted);
                }
            }
            other => break other,
        }
    };
    let elapsed = started.elapsed().as_secs_f64();
    for record in session.debug_records() {
        let text = String::from_utf8_lossy(record);
        for character in text.chars() {
            if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
                print!("{}", character.escape_default());
            } else {
                print!("{character}");
            }
        }
    }

    print_run_report(&session, &samples, elapsed);
    for (start, count) in &peek {
        println!("peek {start:#x} (+{count} u64):");
        for index in 0..*count {
            let address = start.wrapping_add(index * 8);
            match session.memory().read_u64(address) {
                Ok(value) => println!("  {address:#010x}: {value:#018x}"),
                Err(_) => println!("  {address:#010x}: <unmapped>"),
            }
        }
    }
    if watch.is_some() {
        println!(
            "watched writes ({} recorded, cap 4096):",
            session.watch_log().len()
        );
        for record in session.watch_log() {
            let value: String = record
                .bytes
                .iter()
                .rev()
                .map(|b| format!("{b:02x}"))
                .collect();
            println!(
                "  inst={} thread={} pc={:#x} lr={:#x} addr={:#x} len={} value=0x{}",
                record.instruction,
                record.thread,
                record.pc,
                record.lr,
                record.address,
                record.bytes.len(),
                value
            );
        }
    }
    for (start, count) in &disasm {
        println!("disassembly {start:#x} (+{count} instructions):");
        for index in 0..*count {
            let address = start.wrapping_add(index * 4);
            match session.memory().read_u32(address) {
                Ok(word) => println!(
                    "  {address:#010x}: {word:08x}  {}",
                    emulator_a64::disassemble(address, word)
                ),
                Err(_) => println!("  {address:#010x}: <unmapped>"),
            }
        }
    }

    match outcome? {
        RunOutcome::Exited => {
            println!(
                "Guest process exited successfully after {} instructions.",
                session.instructions()
            );
            Ok(())
        }
        RunOutcome::BudgetExhausted => {
            println!(
                "NSP prototype reached instruction step budget ({}) at PC={:#x}",
                session.instructions(),
                session.cpu().pc
            );
            Ok(())
        }
        RunOutcome::Blocked => Err("guest process is blocked waiting for kernel resources".into()),
    }
}

/// Host-side diagnostics after a commercial-title run: where the guest is, what it
/// last asked the kernel for, and where the sampled time went.
fn print_run_report(
    session: &Session,
    samples: &std::collections::BTreeMap<u64, u64>,
    elapsed: f64,
) {
    let cpu = session.cpu();
    let kernel = session.kernel();
    let steps = session.instructions();
    println!("--- run report ---");
    println!(
        "steps={steps} elapsed={elapsed:.2}s rate={:.1} MIPS state={:?}",
        if elapsed > 0.0 {
            steps as f64 / elapsed / 1e6
        } else {
            0.0
        },
        session.state()
    );
    println!(
        "svc_count={} last_svc={:#x} last_svc_pc={:#x} ipc_count={} live_handles={}",
        kernel.svc_count(),
        kernel.last_svc_id(),
        kernel.last_svc_pc(),
        kernel.ipc_count(),
        kernel.live_handle_count()
    );
    println!("threads (saved context; current = live registers below):");
    let current = kernel.current_thread_id();
    for t in kernel.thread_summaries() {
        let [a0, a1, a2, a3] = t.args;
        println!(
            "  {}{:<3} prio={:<2} core={}/{:#x} pc={:#x} lr={:#x} fp={:#x} x0..3=({a0:#x}, {a1:#x}, {a2:#x}, {a3:#x}) {}",
            if t.id == current { '*' } else { ' ' },
            t.id, t.priority, t.ideal_core, t.core_mask, t.pc, t.lr, t.fp, t.state
        );
    }
    // Resolve WaitSynchronization handle lists of blocked threads.
    for t in kernel.thread_summaries() {
        let (pc, [_, list, count, _]) = if t.id == current {
            (cpu.pc, [cpu.x[0], cpu.x[1], cpu.x[2], cpu.x[3]])
        } else {
            (t.pc, t.args)
        };
        let svc_word = session.memory().read_u32(pc.wrapping_sub(4)).unwrap_or(0);
        if svc_word != 0xd400_0301 || count == 0 || count > 64 {
            continue;
        }
        for i in 0..count {
            if let Ok(h) = session.memory().read_u32(list + i * 4) {
                let what = kernel
                    .describe_handle(h)
                    .unwrap_or_else(|| "invalid".into());
                println!("  thread {} waits on handle {h:#x}: {what}", t.id);
            }
        }
    }
    println!("recent supervisor calls (oldest first):");
    for record in kernel.svc_records() {
        let [a0, a1, a2, a3] = record.arguments;
        match record.results {
            Some([r0, r1, r2]) => println!(
                "  #{:<4} svc {:#04x} pc={:#x} in=({a0:#x}, {a1:#x}, {a2:#x}, {a3:#x}) out=({r0:#x}, {r1:#x}, {r2:#x})",
                record.sequence, record.id, record.pc
            ),
            None => println!(
                "  #{:<4} svc {:#04x} pc={:#x} in=({a0:#x}, {a1:#x}, {a2:#x}, {a3:#x}) REJECTED",
                record.sequence, record.id, record.pc
            ),
        }
    }
    println!("recent IPC messages (oldest first):");
    for record in kernel.ipc_trace() {
        let kind = match (record.control, record.tipc) {
            (true, _) => "control",
            (_, true) => "tipc",
            _ => "cmif",
        };
        let command = record
            .command_id
            .map_or_else(|| "close".to_string(), |id| format!("cmd {id}"));
        let object = record
            .domain_object
            .map_or_else(String::new, |id| format!(" object={id}"));
        let outcome = record.result.map_or_else(
            || "REJECTED".to_string(),
            |code| format!("result={code:#x}"),
        );
        let [a, b, x, c] = record.buffers;
        println!(
            "  #{:<4} handle={:#x} {} {kind} {command}{object} data={} buffers(a/b/x/c)={a}/{b}/{x}/{c} copy={} {outcome}",
            record.sequence,
            record.handle,
            record.service.rsplit("::").next().unwrap_or(record.service),
            record.request_bytes,
            record.copy_handles
        );
        {
            let words: Vec<String> = record.words.iter().map(|w| format!("{w:08x}")).collect();
            println!("        words: {}", words.join(" "));
        }
    }
    println!(
        "pc={:#x} sp={:#x} lr={:#x} tpidrro_el0={:#x} tpidr_el0={:#x} nzcv={:?}",
        cpu.pc, cpu.sp, cpu.x[30], cpu.tpidrro_el0, cpu.tpidr_el0, cpu.nzcv
    );
    for row in 0..8 {
        let base = row * 4;
        let cells: Vec<String> = (base..(base + 4).min(31))
            .map(|index| format!("x{index:<2}={:016x}", cpu.x[index]))
            .collect();
        println!("  {}", cells.join(" "));
    }
    // Frame-pointer backtrace: walk the AAPCS64 x29 chain ([fp] = caller fp,
    // [fp + 8] = return address). Stops on unmapped, misaligned or
    // non-ascending frames so a corrupt chain cannot loop.
    println!("backtrace (frame pointers):");
    println!("  #0  pc={:#x} lr={:#x}", cpu.pc, cpu.x[30]);
    let mut fp = cpu.x[29];
    for depth in 1..=24 {
        if fp == 0 || fp % 16 != 0 {
            break;
        }
        let (Ok(next), Ok(ret)) = (
            session.memory().read_u64(fp),
            session.memory().read_u64(fp + 8),
        ) else {
            break;
        };
        println!("  #{depth:<2} fp={fp:#x} ret={ret:#x}");
        if next <= fp {
            break;
        }
        fp = next;
    }
    println!("code around pc:");
    for offset in -8i64..=8 {
        let address = cpu.pc.wrapping_add((offset * 4) as u64);
        match session.memory().read_u32(address) {
            Ok(word) => println!(
                "  {}{address:#010x}: {word:08x}  {}",
                if offset == 0 { ">" } else { " " },
                emulator_a64::disassemble(address, word)
            ),
            Err(_) => println!("   {address:#010x}: <unmapped>"),
        }
    }
    let total: u64 = samples.values().sum();
    if total > 0 {
        let mut hot: Vec<(u64, u64)> = samples.iter().map(|(pc, count)| (*pc, *count)).collect();
        hot.sort_by(|left, right| right.1.cmp(&left.1));
        println!(
            "pc samples: {total} taken, {} distinct; hottest:",
            hot.len()
        );
        for (pc, count) in hot.iter().take(12) {
            println!(
                "  {pc:#010x}: {count} ({:.1}%)",
                *count as f64 * 100.0 / total as f64
            );
        }
    }
    println!("--- end report ---");
}

fn read_image(path: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take((MAX_IMAGE + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > MAX_IMAGE || bytes.len() % 4 != 0 {
        return Err(
            "raw A64 image must be nonempty, at most 16 MiB, and a multiple of 4 bytes".into(),
        );
    }
    Ok(bytes)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
