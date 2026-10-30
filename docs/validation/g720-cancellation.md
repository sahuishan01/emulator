# Supplied physical ARM64 cancellation result

The supplied 0.6.0-dev report (schema 2/workload 6, engine suite 3) passes all
six guest programs and six Vulkan cases on realme RMX5061 / Mali-G720-Immortalis
MC12 / Android 16 API 36. Raw JSON remains ignored at
`reports/physical-g720/v0.6.0-dev-report.json`. SHA-256:
`749be29ed38cb9fd15d25f494be42bc0b66bef64b088a60df78d16a0886b409c`.

Cancellation matches the oracle: 106 instructions, two threads, four switches,
two blocking waits, two interruptions, zero expired blocked waits, 1091 reference
ns, expected debug output and zero handles at exit. All errors are empty; prior
guest counts are unchanged. This closes the device checkpoint in
[android-cancellation.md](android-cancellation.md).

Vulkan 1.3.247 / raw driver 184553472 and device identity match the prior report.
All six host GPU checks pass without native BC. Validation is disabled; zero
reported errors is not a validation-layer pass. This is user-supplied evidence,
not an observed physical ADB session. It does not establish the newer GPU-core
checks, NV service submission, method/shader execution, presentation, sustained
performance or commercial gameplay. Virtual reference time is not console timing.
