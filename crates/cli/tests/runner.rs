use std::process::Command;

#[test]
fn demo_executes_the_guest_program() {
    let result = Command::new(env!("CARGO_BIN_EXE_emulator-cli"))
        .arg("demo")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(output.contains("after 24 instructions"));
    assert!(output.contains("subroutine result=42"));
    assert!(output.contains("no OS service was executed"));
}

#[test]
fn a_nonterminating_guest_cannot_bypass_the_step_budget() {
    let image = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/a64/spin.bin");
    let result = Command::new(env!("CARGO_BIN_EXE_emulator-cli"))
        .args(["run-raw", image, "--max-steps", "5"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("step budget exhausted"));
}

#[test]
fn nro_command_executes_the_loaded_homebrew() {
    let image = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/homebrew/hello.nro"
    );
    let result = Command::new(env!("CARGO_BIN_EXE_emulator-cli"))
        .args(["run-nro", image])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(output.contains("Hello from original Switch homebrew!"));
    assert!(output.contains("Guest process exited after 5 instructions"));
}

#[test]
fn ipc_guest_runs_through_the_same_public_nro_command() {
    let image = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/homebrew/ipc-counter.nro"
    );
    let result = Command::new(env!("CARGO_BIN_EXE_emulator-cli"))
        .args(["run-nro", image])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout)
        .contains("Guest IPC counter reached 42; handles closed."));
}
