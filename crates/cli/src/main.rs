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

fn load_keys(keys_path: Option<&str>, titlekeys_path: Option<&str>) -> Result<emulator_crypto::keys::KeyFile, Box<dyn std::error::Error>> {
    let mut keys = if let Some(path) = keys_path {
        let content = std::fs::read(path)?;
        emulator_crypto::keys::KeyFile::parse(&content)?
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
        let content = std::fs::read(path)?;
        let title_keys = emulator_crypto::keys::KeyFile::parse(&content)?;
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

    let file = file.ok_or("usage: emulator-cli run-nca FILE [--keys KEYFILE] [--titlekeys TITLEKEYS] [--max-steps N]")?;
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
        RunOutcome::Blocked => {
            Err("guest process is blocked waiting for kernel resources".into())
        }
    }
}

fn run_nsp(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
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

    let file = file.ok_or("usage: emulator-cli run-nsp FILE [--keys KEYFILE] [--titlekeys TITLEKEYS] [--max-steps N]")?;
    if budget == 0 || budget > 10_000_000 {
        return Err("step limit must be between 1 and 10000000".into());
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
        println!("Parsed CNMT metadata: Title ID {:#018x}, Version {:#x}, {} content records",
            cnmt.title_id, cnmt.version, cnmt.contents.len());
        for content in &cnmt.contents {
            println!("  Content {:?}: {} ({} bytes)",
                content.content_type, content.nca_filename(), content.size);
        }
    }

    let mut session = Session::from_nsp(&nsp_bytes, &mut keys)?;
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
                "NSP prototype reached instruction step budget ({}) at PC={:#x}",
                session.instructions(),
                session.cpu().pc
            );
            Ok(())
        }
        RunOutcome::Blocked => {
            Err("guest process is blocked waiting for kernel resources".into())
        }
    }
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
