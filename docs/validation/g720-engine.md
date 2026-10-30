# Supplied physical ARM64 original-engine result

The user supplied the 0.4.0-dev Android report, schema 2/workload 4, with all
four original-engine cases and all six native Vulkan cases passing. This is
user-supplied device evidence, not an ADB session observed on the build host.
The raw JSON remains ignored at `reports/physical-g720/v0.4.0-dev-report.json`.
SHA-256: `819269cac558eecb292cfa966d48e22db51e9e61417b03cc88d4fea19a5babca`.

| Item | Reported result |
| --- | --- |
| Phone / GPU | realme RMX5061 / Mali-G720-Immortalis MC12 |
| Android / API | 16 / 36 |
| Engine host / guest architecture | aarch64 / aarch64 |
| CPU backend / engine suite | original-interpreter / 1 |
| Debug-output/exit NRO | Passed, 5 instructions / 0 IPC calls |
| Service manager/counter NRO | Passed, 384 instructions / 5 IPC calls |
| Cloned-session lifetime NRO | Passed, 529 instructions / 7 IPC calls |
| Event signaling/polling NRO | Passed, 87 instructions / 0 IPC calls |
| Handles remaining after each guest exit | 0 |
| Six native GPU checks | All passed, unchanged workload 3 |
| Vulkan / raw driver version | 1.3.247 / 184553472 |
| Vulkan validation | Disabled |

Each engine case reports the expected debug output and an empty error. The
device/driver identity matches the previous physical 0.3.0-dev report. This
confirms the original Rust interpreter/kernel/IPC checks execute through the
ARM64 Rust/C++/JNI boundary on the target phone. It closes the physical
checkpoint described in [android-engine.md](android-engine.md).

The interpreter executes guest instructions even though host and guest share
the AArch64 ISA. These results do not validate native guest execution, thread
scheduling added after this build, guest graphics, full homebrew compatibility,
sustained performance or Z-A gameplay. Zero reported Vulkan errors with the
layer disabled is not a validation-layer pass. The newer scheduling fixture
needs its own physical report; the unchanged GPU suite is already verified.
