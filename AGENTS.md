# Original emulator project

- The user explicitly wants an emulator written from scratch. Implement original emulation code; do not fork, link, import, or copy an existing emulator core.
- `upstream/` contains untouched reference checkouts from an earlier approach. It is not a dependency or the implementation baseline. Keep it out of builds and version control.
- Read README.md, HANDOFF.md and docs/architecture.md for scope. Use docs/design.md for runtime contracts and docs/implementation-plan.md for milestone dependencies and acceptance evidence.
- Implement and test guest instruction behavior independently of the host CPU. Unsupported instructions and services must fail explicitly; never silently pretend they worked.
- Keep CPU, memory, loader, kernel/services, guest GPU and host graphics responsibilities separate. Add modules when implemented; avoid placeholder compatibility claims.
- Primary hardware target: Realme GT 7 Dream Edition / Immortalis-G720. Long-term target game: Pokémon Legends: Z-A. A host microprogram test is not Switch or Android gameplay validation.
- Keep Vulkan decisions capability-driven and record physical-device evidence separately from host tests.
- The user requested autonomous progress: after verifying a checkpoint, continue to the next authorized issue. Do not repeatedly ask for another go-ahead.
- Keep reports, game files, firmware, keys and private captures out of Git. Never copy secret material from global instructions into this project.
