.text
    mrs x0, tpidrro_el0
    mrs xzr, tpidrro_el0
    svc #0x42
    mrs x1, midr_el1 // Privileged identification register: must fault at EL0.
