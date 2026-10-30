.text
    mrs x0, tpidrro_el0
    mrs xzr, tpidrro_el0
    svc #0x42
    mrs x1, cntvct_el0 // Explicitly unsupported: never expose host time.
