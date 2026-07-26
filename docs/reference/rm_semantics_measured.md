# RM semantics — measured and source-verified

**What this file is.** A *reference sheet* of NVIDIA RM/UVM behaviours that this project has
either read out of the open kernel modules or **measured on real hardware**, with the citation
next to each fact. It exists because these facts are load-bearing for several independent
design decisions and were previously only recoverable by reading a 3 500-line contact log in
`../design/l1_concurrency.md`. **The reasoning lives in the design docs; the facts live here.**

**How to use it.** When a design doc needs "what does RM actually do", cite a row here rather
than re-deriving it. When a row is wrong, fix it here and the design docs inherit the fix.
When something is *not* here, the standing rule applies: **do not invent NVIDIA/RM/GSP
behaviour** — read `ogkm`, read gVisor's `nvproxy`, or run a bench experiment, and write the
answer down. Every row is tagged:

- **[src]** — read from `ogkm` (the open NVIDIA kernel modules) or from gVisor.
- **[measured]** — observed on real hardware by this project, with the method stated.
- **[inferred]** — a conclusion drawn from the two above. Marked so it can be attacked.

---

## ⚠️ 0. The version caveat — read this before quoting any number

| | |
|---|---|
| `ogkm` checkout every **[src]** row below is read from | **610.43.02** (`research_clones/ogkm/version.mk:1`) |
| driver the bench GPU actually runs | **580.159.04** (RTX 3060, GA106) |

The **locking architecture** (per-client `RS_LOCK_CLIENT` write locks, one global API lock,
the GSP RPC under both) is stable across these versions, and the gVisor corroboration
(§1, production, its own version matrix) is independent evidence of that. But **any specific
default, mask, or line number must be re-checked against the running driver before it is
written down as fact**, and any *number* quoted from `ogkm` (`apiLockMask`'s default, a
timeout constant) is a 610-series number being used as a 580-series prior. Where a row's
correctness depends on the exact value rather than the shape, it says so.

This is the single most likely way a row here goes quietly stale.

---

## 1. ★ RM serializes every ioctl per client — the pool buys latency isolation, not parallelism

**[src]** Every resource-server entry point reachable from an ioctl takes the per-client lock
in `LOCK_ACCESS_WRITE`:
`ogkm: src/nvidia/src/libraries/resserv/src/rs_server.c:778, :1143, :1503, :1923, :2009,
:2131, :2218, :2546`, and alloc *asserts* it at `:786-788`.

**[src]** The **only** client-**READ** site in the whole driver is kernel-internal and not
reachable from an ioctl at all: `nvGpuOpsGetExternalAllocPtes`
(`ogkm: .../rmapi/nv_gpu_ops.c:4674-4676`, UVM). NVIDIA special-cased exactly one hot path to
get same-client concurrency — which is strong evidence about the general case.

**[src]** Alloc and free additionally take the **global** API lock in WRITE. There is one
`g_RmApiLock` (`.../rmapi/rmapi.c:53-58`, `:535`); the default `apiLockMask` is
`NVBIT(RS_API_CTRL)` only (`.../core/system.c:423`), so `serverAllocResourceLookupLockFlags`
and its free equivalent override read-only back to WRITE
(`.../rmapi/alloc_free.c:1714-1718`, `:1746-1748`). Only `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR`
escapes. **The API lock is held across the GSP RPC** (`.../gpu/gsp/kernel_gsp.c:398`, `:2954`).

**[src, production]** gVisor corroborates independently: `nvproxy` holds a per-client
exclusive mutex across the host ioctl, under real CUDA
(`gvisor: pkg/sentry/devices/nvproxy/frontend_unsafe.go:367-381`).

> **[inferred] Consequence.** A pool of N workers on one RM client buys **~nothing on the
> wire**. Its value is *liveness and latency isolation*: a six-second verb must not make a
> sibling thread's independent verb appear to hang. Sizing a pool to the vCPU count implies a
> scaling relationship that does not exist — past the point where one slow verb cannot hide a
> fast one, extra workers are extra host threads queued in **D state** on the same
> uninterruptible `down_write`.

Design consequences: `../design/l1_concurrency.md` §7.2 (pool sizing, `DEFAULT_POOL_WORKERS`)
and §12.26 (where this correction was made).

---

## 2. ★ RM's waits are UNINTERRUPTIBLE — so an interrupted alloc almost certainly COMPLETED

**[src]** Three independent waits, none of them interruptible:

| wait | mechanism | citation |
|---|---|---|
| the global API lock | `down_write` (no `_killable`, no `_interruptible`) | `ogkm: kernel-open/nvidia/os-interface.c:330-338` |
| the GSP RPC reply | busy-poll loop with **no signal check** | `ogkm: .../gpu/gsp/kernel_gsp.c:2963-3060` |
| client drain at free | bare `while (refCount > 1)` spin | `ogkm: .../resserv/src/rs_server.c:3164-3168` |

**[src]** The GSP RPC timeout is 4 s (`defaultus`, `.../os/os.c:2136-2139`) × 1.5 = **6 s**
(`.../gpu/gsp/kernel_gsp.c:2927`). **RM's own answer at expiry is to keep going** — *"Today,
we will soldier on if GSP times out"* (`:2999-3002`) — escalating to `gpuMarkDeviceForReset`
plus an Xid (`:2772-2792`).

> **★ [inferred] The correctness statement, not a performance one: an interrupted
> `NV_ESC_RM_ALLOC` almost certainly COMPLETED**, because RM had no interruptible point at
> which to abandon it. A forwarder that treats `EINTR` as "it did not happen" leaks a host
> object it can no longer name.

**[inferred] And it re-reads a C-era measurement.** The C recorded a *"~3.4–3.5 s bounded
EINTR unwind measured on RTX 3060"* (`C: docs/design/signal_interrupt_delivery.md`). There is
no unwind: that was **RM's own 4 s timeout elapsing**. Anything sized against "3.5 s" — a
watchdog budget, a wedge detector — is sized against a number that means the opposite of what
it appears to mean. `VERB_BUDGET` must exceed 6 s *plus* API-lock queueing behind other
clients (`../design/l1_os_shell.md` §7.5).

**Still owed a bench measurement** (G4's open question): whether the object allocated by an
interrupted alloc can be reconciled at all, or only reclaimed by isolate death.

---

## 3. ★★ UVM has exactly ONE RM client per module load — not one per process

**[measured]** RTX 3060, driver **580.159.04**, 2026-07-25. `nvUvmInterfaceSessionCreate`
fires **exactly once per `nvidia_uvm` module load**.

```
NVSESS SESSION_CLIENT=0xc1d00069        <- once per module load
 82 dups pid=A dst=0xc1d00069 src=0xc1d00067
 82 dups pid=B dst=0xc1d00069 src=0xc1d00068     <- SAME destination
```

- A **third** process, run later, joined the same destination. It changes only across a
  module reload.
- Over the whole trace: dups with the session as **source** = 0; dups with a user client as
  **destination** = 0; userspace `NV_ESC_RM_DUP_OBJECT` = 0 — **CUDA never issues one**. A
  strict one-directional star *into* the session client.
- **Only 25 of the 82 dups reach GSP.** Any rule keyed on the GSP wire must therefore be
  correct on that subset alone.

**[src]** Consistent with the source: `nvUvmInterfaceSessionCreate` is called once from
`uvm_global_init` (`ogkm: kernel-open/nvidia-uvm/uvm_global.c:117`, from `module_init`) into
the singleton `g_uvm_global`; every dup lands in it (`.../rmapi/nv_gpu_ops.c:2753-2760`,
`:8444-8450`) with the *user's* client as source.

### ★ The method, recorded because it is reusable

This is not visible by ordinary means, and the next person will otherwise waste the same day:

- **`strace` / `LD_PRELOAD` cannot see these.** They are in-kernel RM calls made by
  `nvidia_uvm` on the process's behalf — **never a userspace ioctl**.
- **ftrace refuses the RM core**: it is compiled `notrace`.
- What worked was a throwaway **`register_kprobe` module** on RM's dup funnel
  `rmapiDupObjectWithSecInfo` and on the GSP wire `rpcRmApiDupObject_GSP` /
  `rpcRmApiAlloc_GSP`.

> **Why it mattered.** The C's stale comment *"UVM's per-process gpu-ops client"*
> (`C: src/qemu/nvkvm_gpu_emul.c:392`) seeded a wrong model that was then encoded in **both**
> the Rust core and its test suite — so **no test could have caught it; the tests asserted the
> same wrong model as the code.** Only measurement finds that class of error. (The C's own
> bench note — a singular *"UVM's RM client (0xc1d00001)"* — was the accurate one.)

Design consequence — the `Proc` grouping rule: `../design/l1_concurrency.md` §12.27.

---

## 4. The client-kind discriminator on the wire — and the three things it is NOT

**[measured + src]** `rpcRmApiAlloc_GSP` with `hClass == NV01_ROOT` carries
`NV0000_ALLOC_PARAMETERS`, whose `processID` is the discriminator:

```
GSPALLOC hClient=0xc1d00067 parm.processID=0x0000dd13      <- process A's pid
GSPALLOC hClient=0xc1d00068 parm.processID=0x0000dd14      <- process B's pid
GSPALLOC hClient=0xc1d00069 parm.processID=0xffffffff      <- UVM session = KERNEL_PID
GSPALLOC hClient=0xc1e0006a..76 parm.processID=0xffffffff  <- other RM-internal clients
```

**[src]** Stamped unconditionally at `ogkm: src/nvidia/inc/kernel/vgpu/rpc.h:67-77` —
`privLevel >= RS_PRIV_LEVEL_KERNEL → processID = KERNEL_PID (0xFFFFFFFF)`, else the client's
own `ProcID`. It is a **declared protocol fact**, available at client-creation time, *before
any dup exists*.

Three non-discriminators, each of which looks usable and is not:

1. **★ The handle VALUE.** The UVM session `0xc1d00069` sits numerically *between* the two
   user clients, on the same `RS_CLIENT_HANDLE_BASE` (`ogkm: .../g_resserv_nvoc.h:173`;
   `RS_CLIENT_INTERNAL_HANDLE_BASE (0xC1E00000)` exists and other kernel clients use it —
   **UVM's session does not**). Keying on the range mis-files the single most important kernel
   client in the system, and would have looked right.
2. **`processName`** is empty in every observed record.
3. **The dup graph itself** — which is the thing the discriminator has to decide about.

**[src] Handle values are per-client and reusable.** RM mints client-scoped handles from one
shared base, so *the same raw handle value is live and different in every other client*. A
handle is never an identity on its own; a free issued into the wrong client's namespace does
not fault, it **destroys a bystander**. (This is why `HostHandle` carries its `IsolateId` —
`../design/l1_concurrency.md` §12.26.)

---

## 5. `deviceInstance` is attacker-controlled, and RM fails open to GPU 0

**[measured]** Trivially attacker-controlled: ~20 lines of raw `NV_ESC_RM_ALLOC` on
`/dev/nvidiactl`, **no patched guest kernel**. Stock userspace never emits one.

**[src]** RM enforces `deviceInstance < NV_MAX_DEVICES (32)` in three places
(`ogkm alloc_free.c:1372-1390`; `device.c:118-129` → `NV_ERR_INVALID_CLASS`;
`device.c:357-368`) — **so a `< 32` cap is not where the risk lives.** The real check is
`osIsGpuAccessible` → `nv_is_gpu_accessible` (`kernel-open/nvidia/nv.c:5904-5910`), which
scans the **host process's fd table**; device allocs go through `/dev/nvidiactl`, which
carries no GPU identity, so `deviceInstance` is the *sole* selector.

**[src] `gpumgrGetPrimaryForDevice` fails open to GPU 0** for an in-range-but-unpopulated
instance (`gpu_mgr.c:688-691`).

**[src]** The same `deviceInstance` **twice under one client is legal on bare metal**
(`device.c:368-380` rejects it only under `IS_VIRTUAL`). **Device-per-client is not 1:1** — an
entitlement check must be a *membership* test and must never drift into a uniqueness one.

Design consequence: cap to the **entitlement**, not to `NV_MAX_DEVICES`
(`../design/l1_concurrency.md` §12.21).

---

## 6. Kernel references into user memory can outlive the owning process

**[src]** A dup'd object is kept alive by refcount: `memCopyConstruct_IMPL`'s
`pHwResource->refCount++` / `memdescAddRef` / `DupCount++`
(`ogkm: src/nvidia/src/kernel/mem_mgr/mem.c:1027-1031`). **Kernel clients hold refcounted
references into USER memory** — that is the whole mechanism by which UVM works.

**[src]** And such a reference genuinely **can outlive the owning process**: `uvm_va_space`
hangs off the **file**, not the process
(`ogkm: kernel-open/nvidia-uvm/uvm_va_space_mm.c:75-81`), and
`UVM_INIT_FLAGS_MULTI_PROCESS_SHARING_MODE` states it outright — resources are freed *"when
the last reference to the file is dropped rather than when this process exits"*
(`ogkm: kernel-open/nvidia-uvm/uvm.h:160-167`; zombie ranges `uvm_va_range.h:265-268`).

> **[inferred] The design rule this forces.** A per-process host lifetime keyed on the guest's
> *client root* would be a use-after-free on exactly this path. Keying it on **live resources
> with attribution by origin** makes the surviving kernel reference keep the owner's isolate,
> arena and published backing alive for precisely as long as RM's own refcount says the object
> lives — no extra machinery.

**[src]** RM's own free order is children-before-parents (`rs_client.c:830-849`,
`rs_server.c:963-981`), and RM auto-unmaps at free — so an unmap-then-free discipline on our
side protects *our* mirror, not RM's.

Design consequences and the measured end-to-end behaviour (alive-and-usable: yes; freed at
refcount 0: **no**): `../design/l1_concurrency.md` §12.33.

---

## 7. Cross-PID dup policy

**[src]** `RS_SHARE_TYPE_PID` is the default `DUP_OBJECT` share policy: a cross-PID dup is
**denied unless opted out** (`ogkm: src/nvidia/src/kernel/rmapi/client_resource.c:219-231`,
`sharing.c:344-353`). UVM's own dups pass
`NV04_DUP_HANDLE_FLAGS_REJECT_KERNEL_DUP_PRIVILEGE` (`nv_gpu_ops.c:2759`, `:8490`).

**[inferred]** Genuine cross-process sharing therefore has to be *declared*, which is what
makes "a user↔user dup edge is the sharing edge" a faithful model rather than a convenient one.

---

## See also

- `mode2_bench_lifecycle.md` — the *C Mode-2 artifact's* measured lifecycle behaviour
  (fn-47, driver restart, kill-mid-ioctl). Different subject, same discipline.
- `../design/l1_concurrency.md` §12.26, §12.27, §12.33 — where these facts were established
  and what each one changed.
- `../design/l1_os_shell.md` §0.2 — the per-claim ground-truth table for the OS shell.
