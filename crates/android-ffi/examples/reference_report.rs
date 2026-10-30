fn main() {
    let report = emulator_android_ffi::emulator_run_reference_guests();
    print!(
        "{}",
        std::str::from_utf8(&report.bytes[..report.length as usize]).unwrap()
    );
}
