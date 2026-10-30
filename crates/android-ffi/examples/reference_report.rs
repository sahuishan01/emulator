fn main() {
    let report = if std::env::args().nth(1).as_deref() == Some("gpu-core") {
        emulator_android_ffi::emulator_run_gpu_core_checks()
    } else {
        emulator_android_ffi::emulator_run_reference_guests()
    };
    print!(
        "{}",
        std::str::from_utf8(&report.bytes[..report.length as usize]).unwrap()
    );
}
