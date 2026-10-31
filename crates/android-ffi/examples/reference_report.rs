fn main() {
    let report = if std::env::args().nth(1).as_deref() == Some("nv") {
        emulator_android_ffi::emulator_run_nv_checks_v2()
    } else if std::env::args().nth(1).as_deref() == Some("gpu-core") {
        emulator_android_ffi::emulator_run_gpu_core_checks_v2()
    } else {
        emulator_android_ffi::emulator_run_reference_guests_v2()
    };
    print!(
        "{}",
        std::str::from_utf8(&report.bytes[..report.length as usize]).unwrap()
    );
}
