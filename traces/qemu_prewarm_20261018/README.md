# L6: whole-RAM prewarm and the QEMU startup stall

**STATUS: RESEARCH, 2026-10-18. Investigation stopped before a behavior change.
Branch `codex/prewarm-20261018`, base `0452613b5c71cb175271230b3e14c8e3c3eef2db`.
No new VM/GPU run.**

## Hypotheses registered before reading the implementation

Recorded at the start of L6 (2026-10-18 01:52:41 UTC):

- **INFERRED H1:** a whole-guest-RAM registration pins pages while holding the
  QEMU process's mmap lock, indirectly blocking its main loop behind BQL.
  **FALSIFIER:** bounded, short registration calls; a measured short call; or
  registration in another process/mm. A userspace stack alone cannot establish
  the kernel lock's ownership or duration.
- **INFERRED H2:** another realize-path operation causes BQL contention
  independently of prewarm. **FALSIFIER:** a traced dependency chain identifies
  the prewarm registration as the blocker and excludes another BQL holder.
  Simultaneous userspace stacks alone do not establish that chain.

## Findings and the stop

**CODE READ:** `MemPlane::guest_ram_object` maps the entire guest memfd and makes
ONE `HostRm::alloc_os_descriptor(view, 0, len)` call. `len` is the file's metadata
length, not a bounded registration chunk. For the archived Windows runs it is
`0x300000000` (12 GiB). There is no registration work-size constant or yield
between parts of that call. `alloc_os_descriptor` encodes `limit = len - 1`,
uses `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR`, cached noncontiguous PCI memory with
`MAPPING_NO_MAP`, and issues `NV_ESC_RM_ALLOC_MEMORY` on the GPU node.

**CODE READ:** `kf3_realize` spawns `kf3-vamgr` as a thread in the QEMU process.
`Device::va_loop` takes its manager out of `self.va` and drops that guard before
work. It calls `tspace::prewarm` before waiting in epoll. Registration initializes
a `OnceLock<Result<(u32, u64), String>>`; the guest-RAM topology lookup is outside
that initializer, and absence is not cached. The `RamMap` lookup guard does not
span the ioctl. The raw ioctl adapter asserts that no ranked or adapter-leaf
lock is held. None of these checks detects the kernel's process-wide mmap lock.
The object and its pinned pages persist for the VM's lifetime.

**CODE READ:** NVIDIA's OPEN source at tag `595.91.07`,
`kernel-open/nvidia/os-mlock.c`, `os_lock_user_pages`, uses `current->mm`, takes
`nv_mmap_read_lock(mm)`, calls `NV_PIN_USER_PAGES` for `page_count`, and then
unlocks. Its conditional ENOMEM workaround splits a request into groups of
`NV_NUM_PIN_PAGES_PER_ITERATION` pages **inside the same lock interval**. That is
not a release point. This is a reference-source finding, not verification of
the exact loaded host module or its conftest-selected GUP implementation.

**MEASURED (archived S2 dump):** the same snapshot has the main thread waiting
for BQL, a **vCPU thread** in `mprotect -> grow_heap -> ... ->
memory_region_transaction_commit -> acpi_switch_to_modern_cphp`, and `kf3-vamgr`
in `ioctl -> alloc_os_descriptor -> guest_ram_object -> tspace::prewarm`.
The registration is not in a different process. Kernel stacks and lock ownership
were not captured, so the intermediate mmap-lock dependency remains INFERRED.

**MEASURED (archived sampling timestamps):** attach 2 begins 0.715916 seconds
after the sampler's `t0`, and detach 2 is 5.324040 seconds after it; that attach
interval is 4.608125 seconds. The sampling script invokes all-stop GDB. This
is not an undisturbed sample taken at startup +5 s, and the interval is not an
ioctl duration. `t0` itself is when the sampler discovered QEMU's PID, not QEMU's
creation time. Independent non-GDB logs still show multi-second startup work.

**INFERRED verdict:** H1's mechanism is supported by the single large call,
same-process execution and open driver source. Its live lock-hold duration and
causal responsibility for the missing ticks are not proven. H2 is not excluded.

**CODE READ / STOP:** a registration-only chunk loop contradicts the current
single-descriptor contract. The lane's instruction says to stop and write down
options when the proposed change contradicts the code; no production change or
invented latency test has been made. Specifically:

| Consumer | Existing contract that a chunk-only change would break |
|---|---|
| `MemPlane::ram_obj`, `Mirror::ram_obj`, `Spare::ram_obj` | One handle; memfd offset equals object offset |
| `TSpace::build` | One RAM object mapped as one contiguous window; whole-RAM translations use base + memfd offset |
| `kf_mem::ledger::HostVas::map` and `batch::SpaceVerbs::map_row_in` | A desired RAM row names that one object and may cross any proposed chunk boundary |
| `ChanPlane::birth` sysmem USERD and RC notifier bindings | A single object with the whole-memfd offset; descriptor selection and local-offset conversion would both be needed |
| `HostRm::unmap_row` | Exact-start whole unmaps split at VA reservation edges, not arbitrary RAM descriptor boundaries; adding hidden extra map starts would leave mappings behind |

**INFERRED:** bounding descriptor sizes requires an explicit descriptor collection
and coordinated changes to mapping, rollback, unmapping and native bindings.
Merely yielding around the existing one large ioctl leaves its mmap-lock interval
unchanged. Delaying that ioctl until broker attach does not bound the hold either.

## Why prewarm must stay

**MEASURED (prior repository evidence):**
`docs/design/V3_P5_PORT_MAP.md`, item 26, records registration of 2 GiB taking
1,808 ms, with RAM window mapping 70 ms, space creation 2 ms and store mapping
1 ms. Thirty prewarmed boots recorded 1.73–1.80 s of RAM-object work before the
guest; guest-visible mirror creation stayed approximately at or below 0.1 s.
The earlier first mirror cost reached 7.6 s against a 6 s guest RPC timeout;
that extreme did not reproduce on the later box. These are different hosts/runs.

**MEASURED (prior repository evidence):** `docs/design/V3_DISPLAY.md`, section
8.18, records 3,528,088 us registration at 8 GiB and 880,778 us at 2 GiB on the
trusted host. An early GPU copy waited behind that work. This was a separate
experiment from the startup BQL sample.

**INFERRED estimate:** the latter pair is approximately 0.44 seconds/GiB, or
approximately 5.3 seconds at 12 GiB if linear scaling held. This is neither a
prediction interval nor a measured 12 GiB ioctl duration. Prewarm relocates that
cost out of the first guest-held page-directory reply; deleting it would give
up a demonstrated boot fix. Owner ruling AB also requires T-space prewarm and
forbids a mirror-window fallback.

**MEASURED (archived candidate-4 runs, n=3):** H1, H2 and H4 use revision
`0452613b` with empty extra flag lists. Their T-space-ready timestamps are
4.655–4.799 s after device birth (individual values in `evidence.txt`), and the
T-space build itself takes 194.513–214.095 ms. These timestamps include prior
registration and scheduling; they do not directly measure pinning or BQL wait.
No result from this lane is an after/fix result.

## Options for the owner

1. **INFERRED, recommended first:** have the hardware lane capture the kernel
   dependency chain and independent startup metrics using the checklist below.
   Keep the existing prewarm until the replacement can preserve its contracts.
2. **INFERRED:** follow with a tests-first, coordinated segmented-descriptor
   change. Before changing production behavior, document a named per-call work
   bound and the collection's offset, lifetime, map/unmap and rollback contracts.
   Exercise the real production registration driver with a fake registrar whose
   cost scales with bytes, then commit the failing tests separately as EXPECTED
   RED. The monolithic-call mutant and a missing-release/yield mutant must each
   fail at least two tests. Boundary-spanning maps, native USERD/notifiers,
   partial failures and exact-start teardown need their own regression coverage.
3. **INFERRED:** isolation in another process could avoid QEMU-mm contention
   while retaining a monolithic descriptor, but introduces cross-client sharing
   and lifetime/authority obligations. It is not a small scheduling change and
   was not designed or implemented here.

## Hardware handoff checklist — NOT RUN by L6

**INFERRED experiment plan, before any run:** H1 predicts that the baseline's
long registration overlaps a vCPU mmap-write wait while that vCPU holds BQL;
bounded registration should eliminate the single whole-RAM hold. **FALSIFIER:**
kernel evidence identifies a different blocking lock/mm, the BQL holder is
unrelated, or the same stall remains after the actual hold is bounded. H2 predicts
an independent BQL stall; **FALSIFIER:** matching lock-owner/waiter evidence ties
the stall to registration and the controlled change removes that chain.

- Use the shared host lock and the hardware lane's own paths. Run one VM/GPU
  workload at a time; do not displace the owner's VM without that lane's authority.
- Record exact source/binary/module revisions, RAM size, flags and broker build.
  Compare otherwise matching baseline/fix boots, with at least three fresh boots
  per arm and individual values, n and min/max.
- Capture short GDB all-thread samples targeted at QEMU startup +1, +3 and +5 s,
  in separate diagnostic boots if necessary. Record actual attach/detach times.
  Identify the BQL owner and obtain kernel stacks/lock-wait evidence for the
  registrar and mmap writer. A userspace syscall frame alone is insufficient.
- Collect startup performance in separate **unattached** boots. GDB freezes the
  inferior and can manufacture missing ticks; do not mix diagnostic boots into
  the timeout/first-frame comparison.
- Count broker request timeouts per session and locate them in startup-relative
  time. The lane brief reports 40–46, all in approximately the first 5 s; the raw
  counts were not independently recomputed by L6.
- Count tick-source fallbacks caused by 50 ms silence per session (brief's
  baseline approximately 7–15). Keep the existing silence bound unchanged.
- Measure time from a common recorded QEMU start to first **broker-presented**
  frame and first Windows desktop; distinguish broker connect from first frame.
- Record first-use page-directory reply latency, successful T-space construction,
  privileged-only whole-RAM access, Linux no-window mirrors, USERD/notifier
  operation, reset/unload behavior, and map/unmap cleanup. Do not accept a startup
  improvement that restores the old first-use boot timeout or leaks mappings.
- Preserve the lock-step event-ring window before/through/after the stall, with
  any dump overflow/truncation indicated. The brief's 4.27 s gap/14-tick burst
  remains supplied evidence; L6 did not locate and re-read that raw ring dump.

## Verification scope

**MEASURED:** no production files changed; no cargo build/test or mutant run;
no new GPU/VM run (n=0). Tests-before/after and mutation kills are **NOT RUN / not
applicable to this documentation-only stop**, not a green code verdict. An
EXPECTED RED commit was not manufactured for an unimplemented work scheduler.
The exact-stop report records the raw host locations privately; committed
evidence contains no private host paths, binary-derived analysis or binaries.
