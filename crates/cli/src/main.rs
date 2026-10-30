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
        _ => return Err("usage: emulator-cli demo | run-raw FILE [--max-steps N] | run-nro FILE [--max-steps N]".into()),
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
