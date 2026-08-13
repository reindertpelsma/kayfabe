# w274 — RESULT (2): the spin is PROVEN as a distribution, and the ioctl differential is RE-RUN on a same-box reference

**STATUS: LIVE — 2026-08-12.** Two boots, one host reference capture, build rev **`4fd4cf3`**
(w274_pin) and **`8dc28ee`** (w274b_pin), both STAMP GATE PASS, all six ARM ASSERTIONS PASS on
both. Host reference: `nvidia-gpu-passthrough@03b6201`,
`traces/nvdiff_w274/host_vh/` on **`vh` itself** — one GA106, open `580.159.04`, the same chip
and driver the guest targets.

Pre-registration for the spin proof is in `scripts/bench/cup2_hook_procspin.sh`'s own header,
committed at `4fd4cf3` **before** the boot.

---

## PART 1 — THE SPIN PROOF

### ✔ CLAIM 1 — it is `cuCtxCreate`. Confirmed, not re-derived.

`cup2`'s `CK()` prints *after* the call returns. Last line of stdout: `totalMem=11959 MiB`.
Lines announcing `cuCtxCreate` completing: **0**. `CUP2_RC=124` (anchored `^CUP2_RC=`; the
unanchored form still yields the decoy `CUP2_RC=0` from `GCC_CUP2_RC=0`).

### ★★★★★ CLAIM 2 — it is a userspace spin. 24 samples, every thread.

| tid | comm | samples | `syscall=[running]` | in a syscall | `wchan` |
|---|---|---|---|---|---|
| **1552** | `cup2` | 24 | **24** | 0 | `[0]`, state **R** |
| 1553 | `cuda00001400006` | 24 | 0 | **24** | `do_poll.constprop.0` |
| 1772 | `cup2` (helper) | 24 | 0 | **24** | `do_poll.constprop.0` |

The two helpers are **named**, not counted — neither can be reported as the answer. The main
thread was in userspace in **24 of 24** samples over 25.6 s.

### ⊘⊘ AND PRE-REGISTERED REFUTATION **R2 FIRES** — it is NOT syscall-free.

Over the same window, `tid=1552`:

| counter | base | final | Δ |
|---|---|---|---|
| `utime` | 550 | 2499 | **+19.5 s** |
| `stime` | 1591 | 2331 | **+7.4 s** |
| `voluntary_ctxt_switches` | 66 | 66 | **+0** |
| `nonvoluntary_ctxt_switches` | 91 | 162 | +71 |

**~28 % of the thread's time is in the kernel**, and the syscall sampler caught **none** of it
— every one of those calls returns in microseconds. ⇒ *"a userspace spin with no syscalls"*
would have been an **over-claim**, and the sampler alone would have made it. The integral
instrument caught what the point-sampler structurally cannot. **That is why both ran.**

★ **What is NOT refuted — and is exactly what the owner asked.** `voluntary_ctxt_switches`
**+0 over 26 s** is decisive: the thread **never blocks and never yields**. A wait on a futex,
a `poll`, a sleep or any blocking syscall produces voluntary switches; there are none. It is
preempted (+71 involuntary), like any CPU-bound spinner.

> ⇒ **It is a forever spin loop that calls one fast syscall per iteration** (w271 sampled
> `orig_rax=228`, `clock_gettime` — in a VM without a usable vDSO clocksource that is a real
> syscall). **It is not "hanging in a syscall" and it is not blocked in the kernel.** The
> owner's question is answered in the affirmative, with the one correction that "spin" here
> does not mean "makes no syscalls".

### ★★★ CLAIM 3 — the polled words are FROZEN, not merely short.

All **16 slots × 4 words** at `0x2_0440ff00` are **byte-identical across all 24 samples** —
payloads, pads, and the GPU-written timestamps. Not one write landed in 25.6 s.

```
+0xf70  00000005 00000000 00e25340 18cb131e     <- the CE slot; w271 measured the wait wanting 0x45
+0xff0  00000002 00000000 07fb4c00 18cb131e     <- the GR report semaphore; w271's wait wanted 5
```

⇒ **"never moves", not "moves but never reaches the awaited value"** — over this window. The
two are different bugs and this run distinguishes them. Both slots hold exactly the values
`w271` measured, so the standing wanted-values (`0x45`, `5`) apply: **R4 does not fire, the
wait is not satisfied.**
⊘ Coverage: sampling began ~70 s into the boot. The slots are at 5 and 2, not 0, so they *did*
advance earlier — before the window. What is measured is that they are **stopped**, not that
they never moved.

### ⊘ THE BOOT ENDED IN A QEMU ABORT — and it is our own guard, firing correctly.

```
panicked at crates/kayfabe-util/src/lockwitness.rs:152:5:
R1 no-blocking-under-lock violation (l1_concurrency.md §3.3): munmap (dropping a host
mapping) while holding rank(s) [0]
  19: kayfabe_shim_regs_write
thread caused non-unwinding panic. aborting.
```

Triggered on `cup2`'s teardown, from a guest MMIO write, on the `GR-RING-JOIN` path. The
witness is right; the frame cannot unwind, so the process aborts and the VM dies. **Any hook
phase scheduled after a hung CUDA process is torn down is unreachable by construction** — that
is why the differential needed its own boot, extracting the trace *mid-hang*. It also cost this
boot the device's teardown census (the doorbells-served and completions lines are **absent**,
which is not the same as zero).

---

## PART 2 — THE IOCTL DIFFERENTIAL, RE-RUN

### Pre-flight, non-negotiable, done first

`nvd_selftest.sh` → **PASS**: exactly **479** divergences (`dev` vs `ctx`) and exactly **5**
(`ctx` vs `alloc`), and a zero noise floor on all six committed stages.

### The new host reference is on the SAME BOX — and its noise floor is zero

`vh`, native, no QEMU: **578 records × 2 runs**, `rc=0`, `DONE`. `r1` vs `r2`:
**`NO DIVERGENCE (after canonicalisation)`**. ★ This is a better experiment than the committed
reference: that one is a five-GPU rig on the **closed** driver, which is why `CARD_INFO` was
its top divergence by index and was pure environment.

⚠ `vh` has `libcuda` and **no CUDA toolkit**, and the only `cuda.h` on it is the PowerMac ADB
header. Both sides are therefore built against a bundled minimal header, `NVD_MIN_CUDA=1` —
**the same flag on both**, so the binary stays the constant. Real `cuda.h` `#define`s seven
entry points onto `_v2` symbols; a stand-in that declares the short names binds **v1**,
builds, links, runs, and emits a **different ioctl stream** silently. `nvd_capture.sh` now
greps the linker's relocations and refuses the build unless all seven `_v2` names are bound —
verified `ok` on **both** the host and the guest build.

### THE RESULT — ranked BY KIND, never by index

`records: A(host)=578  B(guest)=437   opcode-aligned ratio=0.709`
**429 divergences**, and the first *by index* is `CARD_INFO` — environmental, exactly as the
standing rule predicts. Ranked by kind:

| kind | n | reading |
|---|---|---|
| **EXTRA** | **77** | ★★★ **every single one is `RM_CONTROL cmd=0x20801702`.** Nothing else. |
| **MISSING** | 218 | ⊘ dominated by the **teardown tail** the guest never reaches: `RM_FREE` 93, `UVM_FREE` 27, `RM_UNMAP_MEMORY` 23, `UVM_UNREGISTER_CHANNEL` 16. **Not substantive divergences.** |
| **UNEXPLAINED** (value) | 132 | **all 132 are before index 360**; top: `UVM_MAP_EXTERNAL_ALLOCATION` 36, `UVM_REGISTER_CHANNEL` 32, `cmd=0xc36f0108` 16, `cmd=0x00800292` 12 |
| **STATUS** | 2 | `cmd=0x20810108` and `cmd=0x2080200a` — host `0x0`, guest **`0x56`** |

**The structural divergence point is index 360** (of 578). Records 0–359 are the *same calls in
the same order*.

### ★★★★★ `0x20801702` IS `NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS` — and it is the SYMPTOM

`ogkm-580.159.04:src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080mc.h:176`. The guest calls it
**77 times**; the host calls it **zero**.

⇒ Joined to Part 1, the picture closes: the GR report semaphore at `+0xff0` is **frozen**,
libcuda spins, and each iteration asks RM to *service interrupts* — because from libcuda's
point of view a completion it is owed has not arrived. **`0x20801702 ×77` is not the cause; it
is the frozen semaphore expressed in the control plane.** Native never calls it because the
semaphore lands.

### ⊘⊘ THREE CORRECTIONS TO THE BRIEF AND TO MY OWN FIRST READING

1. **The stale headline said the guest is in lockstep for 221 of 479 (46.1 %) "up to
   `UVM_MAP_EXTERNAL_ALLOCATION`".** Today the guest **calls `UVM_MAP_EXTERNAL_ALLOCATION` 18
   times and `UVM_CREATE_EXTERNAL_RANGE` 18 times** (host: 25 each). It is **not** blocked
   before that verb — it completes 18 of 25 and stalls on the 19th. My own first pass through
   the MISSING list read those 7 absences as "the guest never gets there"; the per-call census
   says otherwise.
2. **The brief said w210 SERVED `0x20801702` and so the old result is stale.** The result is
   stale in its numbers; **the failure mode is unchanged.** The guest still spins on that
   control, 77 times in a 90-second window.
3. **"Lockstep" is about the SEQUENCE, not the contents.** All **132** value divergences lie
   *inside* the matching prefix, from record 1 onward. The guest issues the right calls in the
   right order for 360 records while **already disagreeing about what they return**.

### ★ The two STATUS rows are the sharpest single lead

The standing prior: hardware returns non-OK **exactly once** in the whole program, so every
other `0x56` we emit is a divergence. This capture: host `nonzero rc/status = 1`, guest `= 3`.
The two extra are ours, and both are **early** — long before index 360:

| index | cmd | host | guest |
|---|---|---|---|
| A[41] | `0x20810108` | `0x0` | **`0x56`** |
| A[95] | `0x2080200a` = `NV2080_CTRL_CMD_PERF_BOOST` | `0x0` | **`0x56`** |

⊘ `0x20810108` does not resolve in `ogkm-580.159.04`'s public SDK headers — unresolved, not
absent.

---

## ⊘⊘ COVERAGE — what this differential did NOT see

*(The owner's instruction: an absence claim without a coverage statement is not a finding.)*

**What was covered.** One stage (`ce` — the `cup2` shape, **zero kernel launches**), one run
per side, `/dev/nvidiactl` + `/dev/nvidia0` + `/dev/nvidia-uvm*`, every ioctl's header bytes
**and** out-of-line parameter bytes **on both sides of the call**, plus return value and errno,
plus `mmap`/`munmap` of nvidia fds. Guest: 437 records, 97 distinct ops. Host: 578 records, 109
distinct ops.

**What it structurally cannot see, and none of it is a small omission here:**

- ★★★ **Everything that is not an ioctl.** BAR/MMIO reads and writes, the doorbell, USERD
  `GP_PUT`/`GP_GET`, the pushbuffer, the GPU's DMA writes into guest RAM, interrupt delivery,
  and **the completion plane**. The wall this campaign is at lives there. A control-plane diff
  cannot reach it, and this one's own headline (`MC_SERVICE_INTERRUPTS`) is a *shadow* of a
  data-plane fact, not the fact.
- ★★★ **Truncation on the most important call.** `NVDIFF_MAXBUF=8192` truncated **21 guest and
  28 host records**, of which **18 (guest) / 25 (host) are `UVM_MAP_EXTERNAL_ALLOCATION`** —
  i.e. *every single one*. The 36 UNEXPLAINED diffs on that call are computed over **partial
  buffers**, and the bytes past 8192 were never compared on either side. ⇒ **The call sitting
  at the divergence point is the one the recorder covered worst.**
- **The guest ran 437 records and stopped.** Everything the host does after index 360 is
  unobserved on the guest side; "MISSING" there means "not reached", not "not issued".
- **One workload, one chip, one driver, one run per side.** `cup2`/`nvd_prog` launch no kernel,
  so nothing about `cuLaunchKernel` is measured. The guest side has **no second run**, so it
  has **no noise floor of its own** — only the host's was measured (zero).
- **Ordering between threads** is recorded as a global sequence number, so interleaving is
  captured but timing is not.

⇒ Per the owner's epistemics: this run **did** find differences that matter. But the place it
found them — a control the guest calls 77 times and hardware calls zero times — is a place the
instrument can see *because it is an ioctl*, and the thing that makes it happen is not.

---

## ★ ONE MORE FACT, FREE: THE FAULT IS WORKLOAD-INDEPENDENT

Four boots, four different processes' ASLR slots, **the same fault**:

| boot | workload | fault |
|---|---|---|
| w271_pin | `cup2` | `GRAPHICS HUBCLIENT_FE @ 0x75b2_aee00000 FAULT_PDE VIRT_WRITE` ch `0x9` |
| w274_pin | `cup2` | `… @ 0x746d_dce00000 …` ch `0x9` |
| w274b_pin | **`nvd_prog`** | `… @ 0x7f5f_1ce00000 …` ch `0x9` |

`w274b` used a **different program** and produced the identical engine/client/access/fault-type
on the identical channel, at its own process's CUDA VA. ⇒ The fault is a property of the path,
not of `cup2`.

★ And the window gap is **not fixed**: `0xA200000` (162 MiB) in w271, `0xC200000` (194 MiB) in
w274. **The fault address is not at a fixed offset from `SET_SHADER_SHARED_MEMORY_WINDOW`** —
they are independent objects, which closes the last reading in which they were related.

## ⊘ A CORRECTION I OWE, IN MY OWN GRADER

`w274_run.sh` prints `(w271_pin: 88)` for `OPERAND-PIN`, taken from **w273's summary table**.
w271's own log says **156**. w274's says 223. ⇒ **A number quoted forward from a summary
instead of read from the log** — the exact class the brief warned about, committed by me, in
the instrument. The head of the distribution is identical across both boots (5/1/1 for tokens
`0x7`/`0x8`/`0x9`), and `DOORBELL-XLATE` matches exactly at 88.
