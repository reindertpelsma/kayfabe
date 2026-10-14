# Windows1550: initial USERD admission refusal

STATUS: ANSWERED, 2026-10-12. Failed admission diagnostic, no app/migration verdict.

Exact source30c887c8651c3e79b6b62c6785eda30f443205d1; unchanged production setup
plus KF3_WIN_USER_RING_RELAY=1. Windows signed in and setup reports RTX4070,
580.88/4096MiB. Private USER birth repeatedly clean-refused USER_RING_USERD_ACCESS
Gone. The benchmark did not reach an app verdict or migration. Owner/controller
stopped the campaign after that falsifier; harness143 is intentional termination,
not a timeout/app failure. QEMU stopped ACPI17:46:06UTC, no hostXid in bounded
window, no QEMU remained/DMA-FQ restored. Personal image untouched.

Diagnosis: the initial worker assumed the CPU-local hUserdMemory from channel
alloc was an original GSP MemoryList handle. That assumption is not the ABI
contract (versions.rs decode_channel_userd_mem separates the names explicitly).
Public ogkm595.84 kernel_channel.c2664–2674 fills userdMem from the kernel's
resolved subdescriptor. The same-GPU native capture has no same-client fn4
candidate for the birth USERD handle of the Gravity channel, while later
migration handles have SYSTEM-list registrations (vfio-ring-p0 allocation audit).
Capture is limited, so absence alone is not a nonexecution/lifetime proof.

Correction: bounded512-byte resolved Sysmem source scoped to the exact channel
allocation and birth RAM topology at birth; later migration still requires the
validated original MemoryList lease. Every CPU copy retains channel identity;
graph-backed copies keep graph -> Pt -> RAM. Initial zero remains pending-birth
commit only. New source review/test checkpoint follows. No native privileged
verb, fabricated graph registration, captured address table or relaxed normal
Linux policy.
