# Physical G720 guest draw checkpoint

User-supplied0.18.0-dev report (schema4/workload18), SHA256
`01337aaebec4477635e6473bd53ba8fbe5ba46d809a9acdf9e9cc468b9fd04de`:
**all102 pass**, engine15/NV50/GPU-core3/shader19/Vulkan15. All fields of all four
Rust groups match fresh host references except architecture (aarch64).

The three new guest draw replay checks pass full512-byte comparisons, including
SPH/vertex/draw decoding, translated shader execution, BGRA and scissor cases.
Device: realmeRMX5061/MT6989/Android16 API36; hardware Mali-G720-Immortalis MC12,
Vulkan1.3.247/rawdriver184553472/nativeBC absent. Validation is disabled; zero
reported errors is not evidence from validation layers.

This establishes the bounded0.18 reference/replay subset on the actual phone.
The new0.19 live homebrew/window path still needs its own physical run; no
commercial startup or gameplay compatibility follows. Raw reports remain ignored.
