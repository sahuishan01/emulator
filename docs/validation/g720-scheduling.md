# Supplied physical ARM64 scheduling result

The user supplied a 0.5.0-dev report (schema 2, workload 5, engine suite 2).
All five original guest cases and all six native GPU cases pass on physical
ARM64/Mali-G720. Raw JSON remains ignored at
`reports/physical-g720/v0.5.0-dev-report.json`. SHA-256:
`b4b99761eaad08036678e8a9aa25647f1dbcd7aeee3fa185c14487aad612c5b5`.

| Item | Reported result |
| --- | --- |
| Phone / GPU | realme RMX5061 / Mali-G720-Immortalis MC12 |
| Android / API | 16 / 36 |
| Host / guest architecture | aarch64 / aarch64 |
| Original CPU backend | original-interpreter |
| Scheduling guest instructions | 97 |
| Threads / switches | 2 / 4 |
| Blocking waits / timeouts | 3 / 1 |
| Reference virtual time | 1680 ns |
| Handles remaining after every guest exit | 0 |
| Prior four engine cases | Passed, unchanged instruction and IPC counts |
| Six native Vulkan cases | Passed, unchanged workload 3 |
| Vulkan / raw driver version | 1.3.247 / 184553472 |
| Validation layer | Disabled |

The new guest's trace and debug output match the host/AVD oracle exactly.
The report has empty per-case errors, and device/driver identity matches the
earlier physical 0.4.0-dev report. This closes the device checkpoint in
[android-scheduling.md](android-scheduling.md). It is user-supplied evidence;
no physical ADB session was observed on the development host.

The fixture validates the implemented single-core scheduler, owned contexts/
TLS, event waits, joins and virtual timeout behavior through the ARM64 native
boundary. The clock remains a reference model, not console timing. This does
not validate the newer cancellation case, guest GPU framing/execution, shader
translation, presentation, sustained performance or commercial titles. Zero
reported Vulkan errors with validation disabled is not a validation-layer pass.
