# The region lock without a privilege grant — a candidate study, measured

**What this file is.** The measured detail behind `../design/guest_memory_lock.md`. The owner
ruled that **a mechanism requiring a root-only facility is not acceptable**, because the
project's premise is an unprivileged host process. This file enumerates every mechanism that
could implement the region lock, measures the serious ones, and names the one to build.

**Why it is a reference and not a design doc.** Same rule as `qemu_bql_spike.md` and
`qemu_102_facilities.md`: where this file and a design doc disagree about *what the machine
does*, **this file wins and the design doc gets amended**. It has no authority over what we
*should* do — only over what is *there*. Three claims in `guest_memory_lock.md` and
`l1_os_shell.md` §6.7 were found to be wrong and are corrected in §2.

**Where it was run.** Measured **2026-07-27**.

| machine | kernel | arch | notes |
|---|---|---|---|
| `vh` (vast.ai bench) | **6.8.0-124-generic** | x86_64 | 21 cores, 49 GB; **itself a KVM guest** |
| dev box | **7.0.0-14-generic** | x86_64 | privilege probes only |

Source read at: **Linux `v7.1.0-rc6`** (`research_clones/linux`, HEAD `6f3ed7fec`) and
**QEMU `v10.2.0`** (fresh clone on the bench). Citations carry the tag, per
`qemu_102_facilities.md` §12's rule.

Harness: `~/rlockbench/{rlock.c,guest.S}` on `vh` — a raw `/dev/kvm` VM with a 64-bit
long-mode guest blob, one 16 MiB "main" memslot and one resizable "target" memslot, plus
witness vCPUs. Every routine is one subcommand (`caps flags ro trip insn mprot dirty uffd
pause rate reg block`).

Tags: **[measured]** observed on the named machine at the named **layer**, **[src]** read from
the named file at the named tag, **[inferred]** a conclusion drawn from those.

> ### ★★ The layer discipline, restated because this file exists partly to enforce it
>
> `qemu_102_facilities.md` §11.1 established the rule the hard way: **a KVM-direct measurement
> is not a QEMU measurement.** Every number below is tagged **[KVM-direct]** or **[QEMU]**.
> There is exactly one **[QEMU]** number in this file and it is quoted from
> `qemu_bql_spike.md` §6, not produced here. Everything else is KVM-direct, and where a QEMU
> figure would be needed to decide something, §10 says so instead of guessing.

> **★ Nesting caveat, inherited.** `vh` is itself a KVM guest, so absolute microseconds are
> inflated — **estimated 2–5×, not measured**. Every decision below rests on a **ratio** or on
> a **rate compared against the design's own declared rate band**, both of which survive the
> inflation. The one place an absolute matters (§7.3's write-rate ceiling) is stated as an
> upper bound on cost, i.e. a lower bound on real-hardware headroom.

---

## 1. The requirement, restated before any candidate is judged

From `../design/guest_memory_lock.md` §0, §2 and §4.3, in the order that decides:

1. **The mechanism must BLOCK, not detect.** §2: the lock exists because *"when we read a
   descriptor, resolve it, and then issue a host operation derived from it, copy-once alone
   lets the guest rewrite the descriptor in between and leaves us acting on behalf of a request
   that no longer exists."* §4.3: *"the guest's write lands **after** our hold ends."*
   A mechanism that tells us afterwards that the bytes changed arrives **after** the host
   operation has been issued, and §7.8's conservation ledger says a host RM verb is not
   generally undoable. **Detect-after-the-fact does not satisfy this requirement, and this
   single line eliminates three otherwise attractive candidates (§4 rows D, D′, G).**
   - ⚠️ *The one honest qualification:* if the "act" can be deferred until after a
     revalidation — read, compute, check-not-dirty, then act — then detect **is** sufficient,
     and that is the R5 shape `l1_concurrency.md` already has. It is a *restructuring* of the
     caller, not a lock, and it is row L in §4.
2. **The unit is a registered page range, never the containing window.** GL3/GL4, and
   `l1_os_shell.md` §6.8.1's precondition: a window is deliberately shared across procs, so a
   window-scoped lock is an I-NOAMP violation by construction.
3. **Reads by the guest must stay cheap.** The locked pages are things the guest *uses* (a GSP
   command queue, an instance block); a mechanism that traps guest **reads** as well as writes
   converts a low-rate write cost into a high-rate access cost.
4. **Resolution must never depend on the core** (GL8) and must not park a vCPU behind the
   serialized executor.
5. **It must work on arm64.** `l1_os_shell.md` §6.0's portability contract makes arm64 a
   first-class backend, not a bolt-on.
6. **It must not require a privilege we do not already require.** The owner's ruling, and the
   reason for this file.

**Two boundaries are permanent under every candidate** and are not re-litigated here: the
isolate has a different `mm`, and GPU DMA does not walk our page tables
(`guest_memory_lock.md` §1.3). No mechanism in §4 changes either.

---

## 2. ★★ Three recorded facts that are wrong, corrected before anything is built on them

### 2.1 There is no flags-only read-only flip. KVM rejects it.

`guest_memory_lock.md` §1.5 and `l1_os_shell.md` §6.7 both carry:

> *"★★ [measured] CORRECTION — a FLAGS-ONLY flip is 1.49 µs, not 230–460 µs … it priced the
> RO-memslot fallback as a device-wide stall per lock, when it is a microsecond flip."*

**[measured, KVM-direct, `vh`]** Setting `KVM_MEM_READONLY` on a live memslot, changing nothing
else, returns **`-EINVAL`**:

```
[E1] flags-only KVM_MEM_READONLY set on live slot     : rc=-1 errno=22 (Invalid argument)
[E1] flags-only KVM_MEM_LOG_DIRTY_PAGES on live slot  : rc=0  errno=0  (OK)
```

**[src]** `v7.1.0-rc6 virt/kvm/kvm_main.c:2075-2082` — in the *modify an existing slot* branch:

```c
	} else { /* Modify an existing slot. */
		if (mem->flags & KVM_MEM_GUEST_MEMFD)
			return -EINVAL;
		if ((mem->userspace_addr != old->userspace_addr) ||
		    (npages != old->npages) ||
		    ((mem->flags ^ old->flags) & (KVM_MEM_READONLY | KVM_MEM_GUEST_MEMFD)))
			return -EINVAL;
```

**[src]** The kernel documents the same thing in prose: `Documentation/virt/kvm/api.rst:1416-1418`
says `KVM_MEM_READONLY` *"can be set … to make a **new** slot read-only"*. **[src]** QEMU knows
it and works around it explicitly — `v10.2.0 accel/kvm/kvm-all.c:373-386` re-issues the ioctl
with `memory_size = 0` first, commenting *"This is needed based on KVM commit 75d61fbc."*

**So the RO toggle is a DELETE followed by a CREATE, at both layers, and always has been.**

### 2.2 The measurement behind "1.49 µs" never toggled READONLY, and does not say 1.49 µs

**[src]** The bench binary that produced the figure is `~/kvmslotbench.c` on `vh`. Its
"flags" arm is at `:293-294`:

```c
		rc = set_slot(victim, KVM_MEM_LOG_DIRTY_PAGES, VICTIM_GPA_A,
			      VICTIM_SIZE, victim_mem);
```

It toggles **dirty logging**, not read-only — which is the only flags-only change KVM permits,
and which QEMU only ever issues from `log_start`/`log_stop` (already noted in
`qemu_102_facilities.md` §11.1 item 2).

**[measured, KVM-direct, `vh`, same binary, rebuilt and re-run today]**, default arms
(4 vCPUs spinning, 8 pre-installed slots), columns `min,p50,p90,p99,p999,max,mean`:

```
CSV,spin,4,8,add,500,16.66,46.70,78.16,2963.06,3712.60,3712.60,102.82
CSV,spin,4,8,del,500,29.50,276.96,530.71,4356.27,8290.51,8290.51,468.11
CSV,spin,4,8,flags,500,15.71,72.39,98.72,1229.41,3050.57,3050.57,99.87
CSV,spin,4,8,noop,500,0.82,1.15,1.41,2.19,9.33,9.33,1.20
```

The `flags` row is **72.4 µs p50**. The row whose magnitude is ~1.2–1.5 µs is **`noop`** — an
*identical* re-issue, which KVM early-returns before any work (`kvm_main.c:2088-2089`
`else /* Nothing to change. */ return 0;`). **[inferred]** The 1.49 µs figure is the **no-op**
row read as the flags row. Independently reproduced on the new harness: no-op re-issue
**0.92 µs p50**, dirty-log flags-only **24.2 µs p50** on a 4 KiB slot with *no* vCPUs running.

> **★ Three independent errors compose into one wrong conclusion.** (a) the flip measured was
> not the flip cited; (b) the number quoted was not the number measured; (c) the flip cited
> does not exist. Each alone would be a footnote. Together they resurrected a fallback that is
> **two to three orders of magnitude** more expensive than recorded, and §7.3 of
> `guest_memory_lock.md` was rewritten around the resurrection. This is the *same* failure
> shape as `qemu_102_facilities.md` §11.1 and §6.3.1: **a number in a design doc is a claim
> about an experiment, and it decays exactly like a named API does.**

### 2.3 Spelled the obvious way through QEMU, a read-only RAM region does not lock — it silently discards

`guest_memory_lock.md` §7.3 says the fallback is *"a **read-only memslot** covering the
lock-path pages"*. On QEMU the obvious spelling of that is `memory_region_set_readonly()` on the
RAM region backing our window. **[src] `v10.2.0`, that does not deliver the write to us — it
drops it:**

- `system/memory.c:1302` — every `MemoryRegion` is born with `mr->ops = &unassigned_mem_ops`,
  and a RAM region created by `memory_region_init_ram_ptr` never replaces them.
- `include/system/memory.h:3153-3164` — `memory_access_is_direct` returns **false** for a write
  when `mr->readonly || mr->rom_device`, so the write leaves the memcpy path and is dispatched.
- `system/memory.c:1528-1531` — `memory_region_dispatch_write` finds
  `memory_region_access_valid` false (because `unassigned_mem_accepts` returns `false`,
  `:1352-1357`), calls `unassigned_mem_write` — whose body is **empty** (`:1344-1350`) — and
  returns `MEMTX_DECODE_ERROR`.

> **This is GL3's hazard delivered by the memory API.** The write is excluded (good) and then
> **thrown away and never reported** (fatal). The guest's store is lost; nothing faults;
> nothing logs at default verbosity. A lock built this way would be *correct* in the mock,
> *observably* fine on a bench that never checks the guest's own view of its queue, and would
> corrupt the guest under exactly the workload it was built for.

**[src] The correct QEMU spelling exists and is a first-class, long-standing facility:**
`memory_region_init_rom_device()` / `memory_region_init_rom_device_nomigrate()`
(`include/system/memory.h:1758`, `:1629`). A ROM-device region has **`romd_mode`**: guest
**reads go direct to RAM** (`memory_access_is_direct` returns true for reads) and guest
**writes are dispatched to the region's `MemoryRegionOps.write`** — our callback, on the vCPU
thread. **[src]** The KVM listener installs exactly that as a read-only memslot:
`accel/kvm/kvm-all.c:1511` `bool writable = !mr->readonly && !mr->rom_device;` and `:1517-1520`.

**[src] Caveat that constrains the design:** there is **no `_ptr` variant** of
`memory_region_init_rom_device` at `v10.2.0` — QEMU allocates the `RAMBlock`. The host pointer
is recoverable (`memory_region_get_ram_ptr`), but `guest_memory_lock.md` §1.3's *"every lockable
region is a slice of a window we ourselves reserved"* stops being true for this region, and
`Reservation::map_fixed_in` does not apply to it.

---

## 3. The one distinction that decides the whole study: **retry** vs **emulate**

Every candidate that stops a guest write does it in one of two ways, and the difference is not
a performance detail:

| | **retry** | **emulate** |
|---|---|---|
| mechanism | the fault is resolved in the *primary MMU*; KVM re-enters and the guest **re-executes the same instruction** | KVM **decodes** the faulting instruction and performs the access on the guest's behalf |
| members | uffd-WP, `mprotect` | RO memslot, no-slot |
| depends on the instruction? | **no** — any store, any width, any ISA extension | **yes** — the decoder must understand it |
| depends on the arch? | no | **yes, decisively** — §7 |
| store atomicity preserved? | **yes** | **no** — a `lock` RMW and a wide vector store are decomposed |

**[measured, KVM-direct, `vh`, x86]** The x86 emulator handled every form thrown at it against
a `KVM_MEM_READONLY` slot — this is the *good* news for the emulate family and it is stronger
than expected:

```
  mov q  8B      [RO slot]  TRAPPED  -> KVM_EXIT_MMIO  gpa=0x40000000 len=8 write=1
  mov d  4B      [RO slot]  TRAPPED  -> KVM_EXIT_MMIO  gpa=0x40000000 len=4 write=1
  movsq  8B      [RO slot]  TRAPPED  -> KVM_EXIT_MMIO  gpa=0x40000000 len=8 write=1
  rep movsb      [RO slot]  TRAPPED  -> KVM_EXIT_MMIO  gpa=0x40000000 len=1 write=1
  movdqu SSE     [RO slot]  TRAPPED  -> KVM_EXIT_MMIO  gpa=0x40000000 len=8 write=1
  vmovdqu AVX    [RO slot]  TRAPPED  -> KVM_EXIT_MMIO  gpa=0x40000008 len=8 write=1
  movnti         [RO slot]  TRAPPED  -> KVM_EXIT_MMIO  gpa=0x40000000 len=8 write=1
  lock xchg      [RO slot]  TRAPPED  -> KVM_EXIT_MMIO  gpa=0x40000000 len=8 write=1
  load (read)    [RO slot]  NOT TRAPPED (the read was served from RAM)
```

Two readings, both load-bearing:

- **★ Reads are free.** Requirement 3 in §1 is satisfied by the RO memslot *for free*: the read
  never leaves the guest. This is the property that makes the emulate family viable at all.
- **★ Wide stores are decomposed.** A 16-byte `movdqu` and a 32-byte `vmovdqu` each arrive as
  **8-byte** MMIO transactions. And `lock xchg` arrives as an ordinary MMIO access — i.e. the
  guest's atomic RMW **is no longer atomic** against another vCPU. Under a held region lock
  that is invisible (nothing else may write). Between holds it is a real semantic loss and
  must be declared, not discovered.

**[src]** The emulate family's failure mode is a first-class KVM concept, not a hypothetical:
`KVM_CAP_EXIT_ON_EMULATION_FAILURE` exists and is **[measured]** `= 1` on `vh`; a form the
decoder does not know produces `KVM_EXIT_INTERNAL_ERROR`. **Eight forms passing is evidence,
not a proof** — see §8 item 2.

---

## 4. The candidate table

Cost columns are **[measured, KVM-direct, `vh`, 6.8.0-124]** p50 unless stated, on a **4 KiB**
region — the size band `guest_memory_lock.md` §3.1 actually cares about (a queue page, an
instance block). ✔/✘ under *blocks* is the §1 requirement-1 gate.

| # | mechanism | privilege | granularity | **blocks?** | cost per lock cycle | cost per guest write | arm64 | verdict |
|---|---|---|---|---|---|---|---|---|
| **A** | **uffd-WP** on our own window VMA | **udev rule** on `/dev/userfaultfd` (not root at runtime) | host page | **✔** retry | **6.5 µs** (3.24 arm + 3.23 disarm); +27 ns/page | 0 when unarmed | ✔ (unverified) | **the displaced baseline** — best mechanism, second-best deployment |
| **B** | **★ permanent read-only region + blocking write handler** (QEMU: `rom_device`) | **none** | region (a dedicated always-RO region) | **✔** emulate | **0 µs — the memslot never changes** | **55.6 µs**, ceiling **17 973 writes/s** | **✘ unsound — §7** | **RECOMMENDED on x86_64** |
| **C** | RO memslot **flipped per lock** (`guest_memory_lock.md` §7.3 as written) | none | slot | ✔ emulate | **124.5 µs** (0 vCPU) → **720.3 µs** (4 vCPU); witness vCPU degraded **2.2–9.1×** | 50.0 µs | ✘ | **struck** — §5. And §2.3: the obvious QEMU spelling silently discards |
| **D** | `KVM_MEM_LOG_DIRTY_PAGES` + `KVM_GET_DIRTY_LOG` | none | page | **✘ detect only** | 20.6 + 18.4 µs | 0 (no exit) | ✔ | **struck by §1 requirement 1** |
| **D′** | `KVM_DIRTY_LOG_RING` / `…_ACQ_REL` | none | page | **✘ detect only** | — | 0 (no exit) | ✔ | **struck** — §6.3 |
| **E** | delete the memslot entirely | none | slot | ✔ emulate | ≈ row C (a DELETE + an ADD) | 50 µs, **reads trap too** | ✘ | strictly worse than B and C |
| **F** | `mprotect(PROT_READ)` on our own VMA | none | page | **✔** retry, **but** | 6.1 µs (3.01 + 3.10) | — | ✔ | **so close, and struck** — §6.2 |
| **G** | HW watchpoints (`KVM_SET_GUEST_DEBUG`) | none | ≤ 4 × ≤ 8 bytes | **✘** x86 data breakpoints are *trap*-type: the store completes first | — | — | — | **struck** — detect-only, 4 slots, steals the guest's debug registers |
| **H** | stop-the-world (kick all vCPUs out of `KVM_RUN`) | none | **the whole VM** | ✔ | **15.6 µs** (1 vCPU) → 76.6 (4) → 122 (8) → **6000 (16)** | 0 | ✔ | **struck** — maximal I-NOAMP violation; §6.4 |
| **I** | `KVM_MEM_USERFAULT` + userfault bitmap | none | page | ✔ retry | would be ideal | 0 when unarmed | ✔ | **does not exist** — [src] absent from `v7.1.0-rc6` |
| **J** | `KVM_SET_MEMORY_ATTRIBUTES` / `guest_memfd` | none | page | ✔ | — | — | — | **struck** — [measured] supported attributes on a normal VM = **0**; §6.5 |
| **K** | `memfd` seals (`F_SEAL_WRITE`) | none | whole file | ✔ | — | — | ✔ | **struck** — [inferred] seals are **irreversible** and refuse a live writable mapping; a lock must unlock |
| **L** | *no lock* — restructure to copy-then-revalidate-then-act | none | — | n/a | 0 | 0 | ✔ | **the honest zero row** — §6.6 |

---

## 5. Row C measured — the fallback as written is 2–3 orders of magnitude off

Because the RO bit can only change by DELETE+CREATE (§2.1), a lock cycle is **four** memslot
ioctls: `DELETE, ADD(RO)` … `DELETE, ADD(RW)`. **[measured, KVM-direct, `vh`]**, p50 µs:

| target slot size | arm (0 vCPU) | **cycle (0 vCPU)** | arm (4 vCPU) | **cycle (4 vCPU)** | witness vCPU throughput |
|---|---|---|---|---|---|
| 4 KiB | 60.9 | **124.5** | 347.8 | **720.3** | **6.93× degraded** |
| 64 KiB | 111.5 | 228.7 | 394.4 | 814.1 | 2.20× |
| 1 MiB | 65.8 | 133.8 | 356.4 | 725.9 | 2.35× |
| 16 MiB | 51.8 | 105.4 | 388.2 | 798.8 | **9.12×** |
| 256 MiB | 111.7 | 226.0 | 416.1 | 845.6 | 4.59× |
| 1 GiB | 316.7 | 652.2 | 585.7 | 1207.6 | 2.34× |

p99 of a cycle is **3.6–7.2 ms** in every row.

Three readings:

1. **Size barely matters; vCPU count does.** The cost is dominated by SRCU grace periods and
   the shadow zap, not by the range — so "make the locked region small" does not rescue it.
   That kills the natural rescue attempt before anyone spends a week on it.
2. **★ The witness column is the real verdict.** A vCPU running a tight loop **in a different
   memslot** loses 2.2×–9.1× of its throughput while another thread churns the lock slot. That
   is `l1_os_shell.md` §6.7's I-NOAMP objection reproduced directly: one proc taking a region
   lock is charged to every vCPU in the VM. (The spread is wide because the idle baseline is a
   300 ms sample; the *direction* is unambiguous in all six rows, the magnitude is not.)
3. **Versus row A on the same harness, same day: 720 µs against 6.5 µs — 110×.** And that is
   the KVM-direct comparison; the QEMU path adds a BQL-held memory transaction and a FlatView
   rebuild on top (§8 item 1).

> **Row C is struck.** Not "kept as an expensive fallback" — struck. A fallback that costs
> 0.7 ms and stalls every vCPU per lock is not a degraded mode of a 6.5 µs mechanism; it is a
> different system.

---

## 6. Why each remaining row is struck, in the order someone would propose them

### 6.1 Rows D and D′ — dirty logging cannot block, proven in both directions

**[measured, KVM-direct, `vh`]** With `KVM_MEM_LOG_DIRTY_PAGES` set on the slot, a guest store:

```
[E6] guest store into a DIRTY-LOGGED slot: rc=0 exit=IO   (IO => it reached the instruction AFTER the store)
[E6] target word = 0x1122334455667788                     (the write LANDED)
[E6] bitmap word0 on the FIRST get after the store = 0x0000000000000001   (and it WAS detected)
```

That is a non-vacuous negative result in the sense `testing_doctrine.md` §1 demands: the
detection arm **fires** (bit set), which proves the instrument was live, *and* the write
landed with no userspace exit. Dirty logging is a **reporting** channel.

**[src]** The dirty ring is the same thing with a different transport, and the ring-full exit is
not a per-write handshake: `v7.1.0-rc6 virt/kvm/kvm_main.c:3513-3535` pushes the entry from
`mark_page_dirty_in_slot`, and `virt/kvm/dirty_ring.c:240-241` merely *raises a request*, which
`:252-255` converts into `KVM_EXIT_DIRTY_RING_FULL` at the **next request check** — after the
faulting instruction has retired. **[src]** `Documentation/virt/kvm/api.rst:8700-8709` describes
it as a collection interface and notes userspace must *kick* a vCPU to flush it. There is no
arm/disarm and no per-page userspace acknowledgement, so there is nothing to hold a writer with.

### 6.2 ★ Row F — `mprotect` genuinely works, and QEMU makes it unusable

This is the best privilege-free *retry* candidate and it deserves its full hearing.

**[measured, KVM-direct, `vh`]**, with a warm writable SPTE established first:

```
[E5] warm write exit = IO
[E5] guest store with VMA=PROT_READ, slot=RW: rc=-1 errno=14 (Bad address)
[E5] after restoring PROT_WRITE, re-entering KVM_RUN: rc=0 exit=IO
[E5] target word after resume = 0x1122334455667788   <- the store landed, correctly, on retry
[E5] mprotect -> PROT_READ        p50 = 3.01 µs
[E5] mprotect -> PROT_READ|WRITE  p50 = 3.10 µs
```

**So the mechanism is real:** it blocks, it is page-granular, it is instruction-agnostic
(retry, not emulate), it needs **no privilege whatsoever**, it costs **6.1 µs** per cycle —
within noise of uffd — and the vCPU **resumes correctly**.

**What kills it is the signalling, at the QEMU layer.**

- **[src]** `v7.1.0-rc6 virt/kvm/kvm_main.c:3005-3025` — for a VMA that is neither
  `VM_IO|VM_PFNMAP` nor writable, `hva_to_pfn` yields `KVM_PFN_ERR_FAULT`, and
  `arch/x86/kvm/mmu/mmu.c:3528` turns that into **plain `-EFAULT`** from `KVM_RUN`. It is
  *not* `KVM_PFN_ERR_RO_FAULT` (which is the read-only-**memslot** path, `:3520-3521`), so it
  does not become an MMIO exit.
- **[src]** It also does not become `KVM_EXIT_MEMORY_FAULT`: every caller of
  `kvm_mmu_prepare_memory_fault_exit` is on the private-memory / `guest_memfd` path
  (`arch/x86/kvm/mmu/mmu.c:3538`, `:4601`, `:4608`, `:4687`). A shared memslot with a
  `PROT_READ` VMA reaches none of them.
- **[src]** `v10.2.0 accel/kvm/kvm-all.c:3212-3233` — QEMU's vCPU loop:
  ```c
  if (!(run_ret == -EFAULT && run->exit_reason == KVM_EXIT_MEMORY_FAULT)) {
      fprintf(stderr, "error: kvm run failed %s\n", strerror(-run_ret));
      ret = -1; break;
  }
  ```
  A bare `-EFAULT` is **fatal to the vCPU** and there is no hook to intercept it from a device.

> **Row F would require patching QEMU's accelerator core** — a third deployment requirement,
> strictly worse than the udev rule it was meant to replace, and touching the one file the
> ≥ 10.2 floor decision (`c3ec258`) was taken specifically to stop us from carrying patches to.
> **Struck on deployment, not on mechanism.** Recorded in full because it is the row that will
> be re-proposed, and because it becomes correct the day KVM reports this class of fault as
> `KVM_EXIT_MEMORY_FAULT` — see §8 item 5.

### 6.3 Row H — stop-the-world is honest and unaffordable

**[measured, KVM-direct, `vh`]** time to get *every* vCPU out of the guest after signalling:
**15.6 µs** (1) → 76.6 (4) → 122.1 (8) → **6000 µs** (16). It blocks everything, including
DMA-adjacent guest activity, at whole-VM granularity, for the entire duration of the hold —
and a hold contains a host RM verb, which `rm_semantics_measured.md` §§1–2 measures at up to
**6 s** on a wedged client. Struck.

### 6.4 Row G — hardware watchpoints trap *after* the store

x86 data breakpoints are trap-type: the store retires, *then* `#DB` is delivered. So even
ignoring the four-register limit and the fact that the guest owns its own debug registers,
this is row D with worse ergonomics. Struck by §1 requirement 1.

### 6.5 Row J — memory attributes are not available to a normal VM

**[measured, KVM-direct, `vh`]**:

```
  KVM_CAP_MEMORY_ATTRIBUTES sys = 8   (the kvm==NULL branch: "the kernel knows the concept")
  KVM_CAP_MEMORY_ATTRIBUTES vm  = 0   (this VM: nothing is supported)
  KVM_SET_MEMORY_ATTRIBUTES(PRIVATE) on this VM = -1 errno=22 (Invalid argument)
```

**[src]** `v7.1.0-rc6 virt/kvm/kvm_main.c:2422-2428` — the only attribute is
`KVM_MEMORY_ATTRIBUTE_PRIVATE` and it is offered only when `kvm_arch_has_private_mem(kvm)`.
**★ Note the trap in the system-level answer:** checking the capability on the **`/dev/kvm`
fd** returns `8` because `kvm == NULL` takes the first branch. Checking it on the **VM fd**
returns `0`. A capability probe written against the wrong fd would have reported this row as
available. Struck.

### 6.6 Row L — the zero row, stated so it is a decision and not an omission

`guest_memory_lock.md` GL1 already makes copy-once primary and the lock the exception. The
alternative to *any* mechanism is to restructure the specific read-then-act so that the host
operation depends only on the validated copy, with an R5 revalidation before commit — which is
`l1_concurrency.md`'s existing shape. **It costs nothing, works on every arch, and needs no
privilege.** It does not cover the case the lock exists for — where the host operation's
meaning depends on guest bytes we cannot carry forward — but **every region proposed for
`PageClass::LockPath` should have to argue past this row first**, and today none has been
argued in writing.

---

## 7. ★★ Row B measured — and the arm64 finding that splits the recommendation

### 7.1 What row B is

Do not flip the memslot. **Create the lock-path region read-only once, at realize, and never
change it.** Guest reads are served from RAM with no exit (§3). Guest writes exit to *our*
write handler **on the vCPU thread**; the handler takes the region `Mutex` — blocking that
vCPU for exactly as long as the region is held — and then applies the write to the backing
through our own writable mapping.

On QEMU that is `memory_region_init_rom_device_nomigrate()` with a `MemoryRegionOps.write`
(§2.3), on a region that also carries `memory_region_enable_lockless_io()` so the handler runs
with **no BQL** (`qemu_102_facilities.md` row 1). It therefore **consumes a deployment
requirement we have already paid for** rather than adding one.

### 7.2 It blocks — proven, not assumed

**[measured, KVM-direct, `vh`]** The guest is in a tight store loop against a permanently-RO
slot; on the first MMIO exit the handler simply does nothing for 50 ms:

```
[E11] first store trapped; backing word = 0x0 (still the pre-store value)
[E11] after holding 51.4 ms without answering: backing word = 0x0
      -> UNCHANGED: the guest write did NOT land (BLOCKED)
[E11] after applying + resuming: exit=MMIO, backing word = 0x1122334455667788
```

That is requirement 1 of §1, demonstrated with the hold in the middle rather than inferred from
the mechanism.

### 7.3 What it costs, on the axis GL2 already governs

**[measured, KVM-direct, `vh`]** guest in a tight store loop, handler applies each write on the
vCPU thread:

```
[E9] trapped guest writes serviced: 53920 in 3.00 s = 17973 writes/s (55.64 us each)
[E9] non-MMIO exits: 0
```

**[QEMU]** The only cross-layer check available: `qemu_bql_spike.md` §6 measured a **trapped
MMIO write on a PCI BAR, dispatched to a device handler, on QEMU 9.2** at **50 µs p50 /
79 306 in 5 s = 15 861 writes/s**. That is the *same primitive at the QEMU layer*, and it
agrees with the KVM-direct number to within **12 %**.

> **★ Read that agreement carefully, because it is exactly the trap §11.1 warned about.** It
> does **not** license quoting KVM-direct numbers at QEMU generally. It says that *for this one
> primitive* — a vCPU MMIO write exit dispatched to a handler — the QEMU-added work is small,
> which is unsurprising because §11.1's whole point was that the expensive QEMU additions live
> in the **memory-topology** path (transactions, FlatView, listeners), and row B **never touches
> the memory topology after realize**. The number that would be inflated is row C's, and row C
> is struck.

Now price it against `guest_memory_lock.md` §1.4's own rate table:

| page class | **[measured, earlier round]** guest write rate | cost under row B, at 55.6 µs/write |
|---|---|---|
| GSP command queue | 200–3000 /s | **1.1 % – 17 %** of one vCPU-thread-second (upper bound: nested) |
| page-table pages | ~18 000 /s | **≈ 100 %** — the mechanism's measured ceiling is **17 973 /s** |

> **★★ The mechanism's capacity limit lands exactly on the taxonomy boundary the design already
> drew.** GL2 refuses the page-table class at 18 kHz on cost grounds; row B *saturates* at
> 17 973 writes/s. That is a third independent derivation of the same line (the first being the
> C's 0x110094 vmexit storm, the second §1.4's arithmetic), and it means row B cannot be
> misapplied to a high-rate page without the failure being immediate and obvious rather than
> subtle.

### 7.4 What it loses, stated as capability and not as caveat

1. **`RegionMode::Opportunistic` ceases to exist** on this backend. Row B *is*
   `AlwaysTrapped`, permanently. GL10 (*"passthrough is never load-bearing"*) is satisfied in
   the strongest possible way — there is no passthrough.
2. **Guest store atomicity on lock-path pages is lost** (§3): a `lock` RMW becomes non-atomic
   against another vCPU, and a wide vector store is applied in ≤ 8-byte pieces. Invisible while
   held; real between holds. **This must become a registration-time declaration**, not a note.
3. **The region is QEMU-allocated** (§2.3, no `_ptr` constructor), so it is not a slice of our
   own reservation and `Reservation::map_fixed_in` does not reach it.
4. **The CPU cost is charged to the guest's own vCPU**, as latency on every write, rather than
   to our thread as a lock cost. At the top of the declared band that is ~17 % of one vCPU
   (nested; **[inferred]** 3–8 % on bare metal at 2–5× inflation).
5. **It depends on the x86 emulator understanding the guest's stores.** Eight forms measured,
   not a proof (§8 item 2).

### 7.5 ★★ And on arm64 it is unsound — this is the finding that splits the recommendation

**[src] `v7.1.0-rc6 arch/arm64/kvm/mmu.c:2316-2318`, `:2358`** — a write to a read-only memslot
takes the same path as a write to no memslot: `io_mem_abort()`.

**[src] `v7.1.0-rc6 arch/arm64/kvm/mmio.c:173-188`** — the first thing `io_mem_abort` does:

```c
	if (!kvm_vcpu_dabt_isvalid(vcpu)) {
		...
		if (test_bit(KVM_ARCH_FLAG_RETURN_NISV_IO_ABORT_TO_USER, &vcpu->kvm->arch.flags)) {
			run->exit_reason = KVM_EXIT_ARM_NISV;
			...
			return 0;
		}
		return -ENOSYS;
	}
```

**[src] `Documentation/virt/kvm/api.rst:7096-7112`** explains why: *"for certain classes of
instructions, no instruction decode (direction, length of memory access) is provided, and
fetching and decoding the instruction from the VM is overly complicated to live in the kernel."*

**[inferred, from the Arm ARM's ISV definition and from why arm64 forbids `memcpy()` to device
memory]** the classes with `ISV = 0` include **load/store pair (`STP`/`LDP`), SIMD/FP
load/stores, load/store exclusive, LSE atomics, and `DC ZVA`** — i.e. precisely what a
compiler emits for `memcpy`/`memset` into a queue page. Tagged `[inferred]` because I had no
arm64 host: this is a read of the specification and of the kernel's own justification, not a
measurement.

**[src] And QEMU's response makes it fatal rather than recoverable.** `v10.2.0
target/arm/kvm.c:560-565` enables `KVM_CAP_ARM_NISV_TO_USER` unconditionally, and `:1379-1400`
(reached from `:1498-1501`) handles the resulting exit by setting
`events.exception.ext_dabt_pending = 1` — **injecting an external data abort into the guest.**
On Linux that is a synchronous external abort: an oops, not a retried store.

> **On arm64, a permanently-read-only guest-RAM page is not a lock. It is a page that kills the
> guest the first time the guest driver `memcpy`s into it.** And the same argument strikes rows
> C and E on arm64, since they share the emulate path. Row A (uffd-WP) is untouched, because
> retry never asks what the instruction was.
>
> **[src] A second, narrower arm64 divergence, documented by the kernel** and worth recording
> because it is invisible on x86: `Documentation/virt/kvm/api.rst:1423-1430` — on arm64 a write
> by the **page-table walker** (an A/D-bit update) to a `KVM_MEM_READONLY` slot never produces
> `KVM_EXIT_MMIO`; KVM injects an abort instead, *"because KVM cannot provide the data that
> would be written by the page-table walker."*

---

## 8. ★ Row A re-measured, and the two things that changed about it

Row A is the displaced baseline, so it was re-measured on the same harness on the same day.

**[measured, KVM-direct, `vh`]**

| operation | 4 KiB | 16 MiB | 1 GiB |
|---|---|---|---|
| `UFFDIO_WRITEPROTECT` arm | **3.24 µs** | 111.0 µs | 4976 µs |
| `UFFDIO_WRITEPROTECT` disarm | **3.23 µs** | 104.6 µs | 4877 µs |

The 16 MiB row is **27 ns/page**, which reproduces the 2026-07-26 round's *"≈25 ns/page
marginal"* almost exactly — an independent cross-validation of the earlier bench on a
different harness.

**[measured] What did NOT reproduce: the trapped-write round trip.** The earlier round recorded
**24.8 µs p50**; this harness measures **244 µs p50** with a spinning handler thread and 389 µs
with a `poll`-based one. **[inferred]** The difference is the cross-thread handoff, not the
mechanism: row A resolves a fault on a *different* thread from the vCPU, so its trap latency is
dominated by a scheduler wakeup and is an artefact of the handler's design. **I did not
reproduce 24.8 µs and I do not claim it is wrong** — I claim it is handler-shape-dependent, and
that the *arm/disarm* cost (which is not) is the number the Opportunistic design actually
spends. Note the structural asymmetry this exposes: **row B has no handoff at all** — the trap
lands on the vCPU thread that caused it.

### 8.1 The privilege fact, re-measured on two kernels

**[measured, `vh` 6.8.0-124 and dev box 7.0.0-14, identical output on both]**

```
  /proc/sys/vm/unprivileged_userfaultfd          : 0
  /dev/userfaultfd mode                          : 0600 uid=0 gid=0
  --- as uid=65534 (unprivileged) ---
  syscall userfaultfd(full mode)                 : fd=-1 errno=1  (Operation not permitted)
  syscall userfaultfd(UFFD_USER_MODE_ONLY)       : fd=5  errno=0  (OK)
  open("/dev/userfaultfd")                       : fd=-1 errno=13 (Permission denied)
  open("/dev/kvm")   [control]                   : fd=-1 errno=13 (Permission denied)
```

**[src] `v7.1.0-rc6 fs/userfaultfd.c:2172-2190` vs `:2192-2198`** — the asymmetry is structural,
not incidental:

```c
	if (capable(CAP_SYS_PTRACE))            /* userfaultfd_syscall_allowed() */
		return true;
	return sysctl_unprivileged_userfaultfd;
...
SYSCALL_DEFINE1(userfaultfd, int, flags)
{
	if (!userfaultfd_syscall_allowed(flags))
		return -EPERM;
	return new_userfaultfd(flags);
}

static long userfaultfd_dev_ioctl(struct file *file, unsigned int cmd, unsigned long flags)
{
	if (cmd != USERFAULTFD_IOC_NEW)
		return -EINVAL;
	return new_userfaultfd(flags);          /* NO capability check at all */
}
```

> ### ★★ The briefed premise is half wrong, and the half that is wrong matters
>
> *"uffd is root-only unless an admin installs a udev rule"* — **the second clause is the
> whole fact and the first is misleading.** `/dev/userfaultfd` performs **no capability check
> whatsoever**; the *only* gate is the device node's file mode. So uffd does not require root
> **at runtime**; it requires a **file-permission grant on a device node**.
>
> **And the control line above is the one to sit with:** on both stock hosts, the same
> unprivileged uid is denied **`/dev/kvm`** for exactly the same reason. Taken literally, the
> ruling *"a mechanism requiring root is not acceptable"* would also rule out KVM. The real,
> defensible cost of row A is therefore **not privilege** — it is **a second deployment
> requirement**, in a project that has just spent one on the QEMU floor. That is a good enough
> reason to prefer an alternative, and it is the reason this study takes seriously; it is not
> the same reason as the one briefed.

### 8.2 ★★ Row A's own open question, closed on the way past

`guest_memory_lock.md` §4.4 flags as *"the most likely way this design has a silent hole"*
whether a `MAP_FIXED` placement inside a registered window drops the uffd registration. It is
one experiment and it was cheap, so it was run.

**[measured, KVM-direct, `vh`]**

```
[E10] registered the whole 16777216-byte window for WP
[E10] arm sub-range BEFORE any placement          : rc=0  errno=0  (OK)
[E10] MAP_FIXED a fresh memfd backing over [1M,1M+64K)
[E10] arm the SAME sub-range AFTER the placement  : rc=-1 errno=2 (ENOENT)  <-- registration WAS destroyed
[E10] arm after MAP_FIXED ANON restore            : rc=-1 errno=2 (ENOENT)
[E10] re-REGISTER the placed sub-range            : rc=0  errno=0  (OK)
[E10] arm again after re-registration             : rc=0  errno=0  (OK)
```

**The inference in §4.4 is correct.** Both publication shapes — a memfd placement and the
anonymous `Reservation::restore` — destroy the registration for the sub-range, the subsequent
arm fails **loudly** with `ENOENT` (never silently), and re-registration restores it fully.
§4.4's conservative design is therefore **required, not merely prudent**, and §8 residual 1 is
closed.

---

## 9. Acceptance table

| # | claim | how it was established | verdict |
|---|---|---|---|
| 1 | The requirement is **trap-and-block**, not detect | read from `guest_memory_lock.md` §2/§4.3 + `l1_os_shell.md` §7.8 | **settled** — eliminates rows D, D′, G |
| 2 | There is **no flags-only RO flip** | [measured] `-EINVAL`; [src] `kvm_main.c:2079-2082`; [src] QEMU works around it at `kvm-all.c:373-386` | **settled** |
| 3 | The "1.49 µs flags-only flip" is not that measurement | [src] `kvmslotbench.c:293-294` toggles dirty logging; [measured] re-run gives flags p50 72.4 µs, noop p50 1.15 µs | **settled — the doc is wrong** |
| 4 | RO-flip-per-lock costs 124–720 µs and degrades other vCPUs 2.2–9.1× | [measured, KVM-direct] 6 sizes × 2 vCPU counts | **settled** — row C struck |
| 5 | Dirty logging / dirty ring cannot block | [measured] write landed + no exit + bit set; [src] `dirty_ring.c:240-255` | **settled** |
| 6 | `mprotect` blocks and resumes correctly, but signals as bare `-EFAULT` | [measured]; [src] `mmu.c:3528`, no `KVM_EXIT_MEMORY_FAULT` caller on this path; [src] QEMU `kvm-all.c:3219` | **settled** — row F struck on deployment |
| 7 | A permanently-RO region blocks the writer | [measured] 51.4 ms hold, write did not land, resumed correctly | **settled** |
| 8 | Its ceiling is ~18 000 writes/s at ~56 µs each | [measured, KVM-direct]; corroborated **[QEMU]** by `qemu_bql_spike.md` §6 (15 861/s, 50 µs) to within 12 % | **settled for x86** |
| 9 | Guest **reads** of a RO region are not trapped | [measured]; [src] `memory_access_is_direct` | **settled** |
| 10 | x86 emulates every store form tested against a RO slot | [measured] 8 forms | **evidence, not proof** — §10 item 2 |
| 11 | arm64 **cannot** emulate ISV=0 stores; QEMU injects a guest abort | [src] `mmio.c:173-188`, `api.rst:7096-7112`, QEMU `target/arm/kvm.c:1379-1400` | **settled in source; unmeasured** — §10 item 3 |
| 12 | `memory_region_set_readonly()` on a RAM region silently discards writes | [src] `memory.c:1302`, `:1344-1362`, `:1528-1531` | **settled in source; unmeasured** — §10 item 1 |
| 13 | uffd via `/dev/userfaultfd` has **no** capability check | [src] `fs/userfaultfd.c:2192-2198`; [measured] on two kernels | **settled** |
| 14 | An unprivileged uid is denied `/dev/kvm` on the same hosts | [measured] on two kernels | **settled** |
| 15 | `MAP_FIXED` destroys uffd registration; re-register fixes it | [measured] both placement shapes | **settled — §4.4's open question closed** |
| 16 | `KVM_MEM_USERFAULT` does not exist | [src] absent from `v7.1.0-rc6 include/uapi/linux/kvm.h` | **settled** |
| 17 | Memory attributes are unavailable to a normal VM | [measured] VM-fd cap = 0, `SET` = `-EINVAL`; [src] `kvm_main.c:2422-2428` | **settled** |

---

## 10. What this study does **NOT** establish

Stated so nothing above is over-read.

1. **No QEMU number was produced here.** §2.3's silent-discard finding and §7.1's `rom_device`
   remedy are **source reads at `v10.2.0`, not measurements.** The experiment that settles
   both: build a throwaway PCI device with (a) a `memory_region_init_ram_ptr` region flipped
   read-only and (b) a `memory_region_init_rom_device_nomigrate` region, have a guest store to
   each, and assert that (a) loses the write with no callback while (b) delivers it to
   `MemoryRegionOps.write`. Same shape as the `qemu_bql_spike.md` §3 harness; a day's work.
2. **Eight instruction forms is not the x86 instruction set.** `KVM_CAP_EXIT_ON_EMULATION_FAILURE`
   is `= 1` on the bench precisely because emulation can fail. The experiment that settles it
   is not synthetic: run a **complete Mode-2 guest-driver lifetime** with the real GSP command
   queue in a permanently-RO region and assert **zero** `KVM_EXIT_INTERNAL_ERROR` and zero
   `MEMTX_DECODE_ERROR`. Until that runs, row B is a *recommendation*, not a validated one.
3. **No arm64 host was involved.** §7.5 is a source argument, and the specific ISV=0
   instruction list is `[inferred]` from the Arm ARM rather than measured. The experiment:
   an arm64 KVM host, a guest that `memcpy`s into a `KVM_MEM_READONLY` page, and a check for
   `KVM_EXIT_ARM_NISV`. Also unverified on arm64: that **uffd-WP traps a guest vCPU write at
   all** — the whole of §1.1 of `guest_memory_lock.md` is an x86 measurement, and row A's
   arm64 column in §4 is `[inferred]` from uffd being generic mm code.
4. **The witness-degradation column in §5 has a wide spread (2.2×–9.1×)** because the idle
   baseline is a 300 ms sample taken once per configuration. The *direction* is consistent
   across all six rows; the *magnitude* should not be quoted.
5. **Row F is struck on today's kernel and today's QEMU.** If KVM ever reports a shared-memslot
   write to a non-writable VMA as `KVM_EXIT_MEMORY_FAULT` (the plumbing exists —
   `KVM_CAP_MEMORY_FAULT_INFO`, and QEMU already tolerates that combination at
   `kvm-all.c:3219`), row F becomes the best candidate in the table: privilege-free, retry-
   semantics, arch-portable, 6 µs. **That is one upstream patch away**, and it is worth
   watching rather than forgetting.
6. **Nothing was soaked.** Every number is a microbenchmark of ≤ 3 s or ≤ 2000 iterations.
7. **Multi-process contention is untouched** — the same gap `l1_os_shell.md` §6.7's
   measurement left. Every run here is one guest process' worth of pressure.
8. **The store-atomicity loss (§7.4 item 2) was demonstrated but not exploited.** I showed
   `lock xchg` arrives as an ordinary MMIO access; I did not construct a two-vCPU race that
   observes a torn RMW. If any proposed `LockPath` page turns out to carry a guest-side atomic,
   that experiment becomes mandatory before row B is used on it.
