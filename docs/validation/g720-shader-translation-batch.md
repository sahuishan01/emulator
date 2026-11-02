# Physical G720 shader translation checkpoint

Supplied0.17.0-dev report: schema4/workload17; SHA-256
`8f1eb662a480828c864ee75a4fce827803d0a28e045578d00d63de0c9a69da1a`.
All78 checks pass: engine14/NV34/GPU-core3/shader15/Vulkan12. Every field in all
four Rust groups matches the corresponding host references except architecture
(aarch64). The three translated-shader Vulkan cases pass their full independent
pixel comparisons: RGBA, transformed red/BGRA and ordered green/red shader change.

Device remains realme RMX5061/MT6989, Android16/API36; Mali-G720-Immortalis MC12,
Vulkan1.3.247/raw driver184553472, hardware renderer/native BC absent.
Validation is disabled; zero recorded errors is not a validation-layer run.
This validates our owned raw SM53 subset and Vulkan fixtures on this phone;
it does not establish broad guest shader compatibility or console performance.

The new0.18 guest header/vertex/draw command batch awaits physical evidence.
Guest live rendering/presentation, commercial startup and Z-A remain unfinished.
Raw supplied reports and captures remain ignored.
