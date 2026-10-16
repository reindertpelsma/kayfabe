# W3: Turing seven-minute Windows hold and R34 completion comparison

**STATUS: RESEARCH, 2026-10-16. Written before new rental and hardware runs.**

The previous unprivileged TU106 lease passed Windows580.88 short validation
after generated display vocabulary and C57E shared-semaphore fixes. It did
not establish a seven-minute0TDR hold. Linux raw R34 at unchanged180seconds
timed out, while native R34 passed4seconds and a longer diagnostic guest run
finished201seconds. Models/CPU tests do not establish hardware performance.

H-T7: the shared C57E semaphore model keeps a Windows580.88 desktop and a
hardware D3D workload alive for seven minutes. Falsifier: any System4101TDR,
adapter problem, missing DWM/Explorer/desktop screenshot, D3D/CUDA failure
or failing display realize. Capture guest events and actual driver builds,
source/binary SHA and actual KF3 process environment. Do not count a failed
guest command or absent log as a pass.

H-W1 callback hypothesis/falsifier is preserved in the existing walk-completion
README: remove repeated helper callbacks, record a fresh GPU event outside
graph replay and query it before reading the pinned report. On one rental,
compare the baseline to codex/fix-walk-completion ffd72e3d4 at the unchanged
180second original-depth R34 budget, collect walks/nonstall wakes/traps/time.
No interrupt filtering, test-depth reduction or budget increase. Native first;
if time/wake counts do not improve the hypothesis is falsified. Linux gates,
raw30/30, CUDA4 and broker/Windows regression required before green claims.

Rent one20CPU/64GB RTX2060S KVM offer; exact die remains pending unprivileged
RM verification. At most3h and40USD campaign total; immediately arm175minute
timer. Current preceding Blackwell lease is absent. Only own new ID may be
retired. No owner GPU/VM, no agent forwarding, no secrets. NativeWindows LTSC
reference first, restoreLinux595.91.07, then nestedWindows580.88 and Linux
guest580.159.04. All actual builds must be measured/asserted, no ABI guessing.

Evidence/code split: this branch codex/sweep-turing-hold-20261016 is evidence;
new code-only drafts target claude/gpu-types-20261015. Keep R34/C57E candidates
separate and record exactly which runtime each hardware run uses. Stop/rank
at the wall; no family or multiple-driver acceptance until suites are complete.
