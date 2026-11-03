# Physical G720 depth-clear checkpoint (0.26)

The supplied realme RMX5061 / MT6989 / Android 16 API36 report records
Mali-G720-Immortalis MC12, a physical renderer and all208 checks passing:
engine18, NV50, GPU core60, shader25 and Vulkan55. All four complete Rust
groups match the 0.26 host references after removing only hostArchitecture;
all55 native named results match exactly.

All39 ordered visible patterns pass with the same current app version/run ID,
pixel verification, surface update and one completed/presented frame each.
This includes all16 ordered full/partial/scissored/empty depth-clear scenarios.
The pattern batch completed in62.454seconds; the final stopped scene is38.
Validation layers were disabled, validationErrors0 and error empty. These
results confirm the owned corpus, not general Switch game compatibility.

The report records automatic Downloads/Emulator export, filename
emulator-gpu-report-1793710786261-ed1da340.json and content URI ending1000070204.
The attachment alone cannot prove cache/download byte identity or manual
rotation/sharing behavior. Those have separate Android QA evidence.

Ignored raw copy: reports/physical-g720/v0.26.0-dev-report.json.
Size78905bytes; SHA256
4920b315fea8853cb18ee6864b08155d35144e3a8ddee588330a5c2e9125b19f.
App0.26.0-dev/code29, schema4/workload29/native14.
