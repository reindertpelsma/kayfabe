# PLAN — the MMIO contract, w472

**STATUS: LIVE, 2026-09-11.** Owner's target, verbatim:

> *"get passthrough across bar1/bar2 completely no trap unless its a fault, for bar0 ONLY
> write traps, and get EACH write trap in bar0 in microseconds"*

and the ruling that produced it:

> *"every time you discover so long trap, it isn't one extra case you have forgotten to
> defer, its an invalid fundamental construction of how to deal with any mmio trap in the
> first place"*

⊘ **The record supports the ruling.** w468 (mirror walk), w469 (stale-mirror flood),
w432/w447 (GSP RPC service) and w470 (isolate spawn) were each found separately, fixed
separately, and another appeared behind it. `[measured w471]` **1239 blocking-door reaches
on vCPU threads in ONE boot, across 5 distinct doors.** It was never a backlog of cases.

## The three invariants this plan is graded on

| | target | `[measured w471]` |
|---|---|---|
| BAR1/BAR2 | no trap except a genuine guest fault | demand-paged: a miss per page, 3 syscalls each, on the vCPU |
| BAR0 reads | do not trap at all | trapped; the read path also runs `BarMirror::fill` |
| BAR0 writes | each one microseconds | 129 slow traps, worst 1.88 s |

## The shape a BAR0 write handler is allowed to have

1. classify the write against a small table
2. update O(1) shadow state (so a later read, and a later write, observe it)
3. enqueue
4. wake, only on an empty→non-empty transition
5. return

⊘ **Nothing else.** No syscall, no allocation that can fault, no ranked lock, no host verb,
no page walk, no process spawn. `assert_lock_free` already refuses the lock half at every
blocking door; w471 made the same door refuse the vCPU half. Flipping
`KAYFABE_VCPU_BLOCK_FATAL=1` is what makes this a mechanism instead of a habit.

★ **The passthrough doorbell is the one inline exception** and needs no queue: it is a
single store to an already-established host mapping, no state, no completion. ⊘ Its mapping
must be established off the vCPU beforehand, or the first doorbell faults into mapping work.

★ **PRAMIN is the second, and it is enumerated, not implied** (owner, 2026-09-11): the
window-base write re-points the aperture, the next read must observe it, and there is no
completion for the guest to wait on. Boot-only, so the cost is bounded. It gets a named
`*_on_vcpu` door declaring why, the way `assert_only_ranks` is the named exception to
`assert_lock_free` — never a silent pass.

## Work, in order

### 1. BAR1/BAR2 — populate at the synchronization points, not on a fault
★★★ **The unification.** A BAR1/BAR2 mapping IS page-table state, and page-table state
changes are announced at the three synchronization points (TLB invalidate, RM call, UVM
kernel channel). So the mirror should be populated **by the worker at those points**, where
the refresh already runs and already holds the guest at its own wait primitive. Then a miss
is not a cache miss — it is a page the guest never mapped, i.e. a genuine fault.
⊘ Today `fill` runs on the vCPU from the read and write paths, which is a demand-pager.

### 2. One memslot per window, not per page
`[from nvkvm-pv, `src/qemu/virtio_nvgpu.c`]` it reserves the GPA range with a **reservation
BAR**, then covers it with **one big KVM region over a `MAP_NORESERVE` mmap** and changes
what is inside with `mmap MAP_FIXED`. ⇒ no `KVM_SET_USER_MEMORY_REGION` per page. Ours
installs a slot per page (`install_file_window(gpa, PAGE, …)`), which is the most expensive
of the three syscalls: a memslot update rebuilds KVM's memslot array under RCU.
⊘ We already have the reservation half (`bar_is_unbacked_reservation`, the
`bar1-passthrough`/`bar2-passthrough` QOM properties), so the guest's own PCI enumeration
keeps any other emulated device out of the range. The missing half is the single slot.
⚠ Express it through the VMM abstraction (`kayfabe-vmm-kvm` / `kayfabe-vmm-qemu`), not in
the QEMU shim, or it is not hypervisor-agnostic.

### 3. BAR0 reads — back them with a shadow page so they do not trap
A register read must return a value, so it can never be queued. ⇒ it must never be
*computed*. Back BAR0's readable space with RAM the worker keeps current; the guest's poll
loop then spins on memory with zero exits, and the worker's completion write is what
releases it. ★ This is the completion contract already in force, made cheap.
⚠ Registers with genuine read side effects are an enumerated exception and must be listed
with the argument, not discovered.

### 4. The isolate spawn
`materialize_pending` at the tail of every register write forks a process and blocks in
`recv()` (`[measured w470]`). Move it to the worker; pre-warm at proc creation rather than
lazily. ⚠ TWO paths reach it: this one and `verb_op`'s `FwdFault::IsolatePending` arm
(`device.rs:2796`), which materializes on whatever thread hit it. Both must move.

### 5. Flip the census to fatal
`KAYFABE_VCPU_BLOCK_FATAL=1` in CI once the census is empty. Until then the census line
(`VCPU-BLOCKING …`, which prints its empty arm explicitly) is the gate.

## Grading

Every step: client `W392D_GUEST_OUTCOME=(P)` with `THREADS 8 of 8` and
`MEAN_FALSIFIER=PASS`, plus `VCPU-BLOCKING` strictly shrinking and `slow_traps` strictly
shrinking. ⊘ A step that improves a number and loses the client is not progress.


---

## The order of work, and what "done" means at each step (owner, 2026-09-12)

> *"after raw client slow trap free, get llm slow trap free and only then iterate for llm to
> parity for perf overhead that remains"*

| # | milestone | done when |
|---|---|---|
| 1 | **raw client slow-trap free** | no trap over 1 ms during the client phase, read off `TRAP-PHASES` rather than off a whole-boot maximum; client still `(P)` 8/8 |
| 2 | **LLM slow-trap free** | same bar under the LLM workload, which drives ~190 launches per token and is a different trap regime entirely |
| 3 | **LLM to parity** | only then chase the residual overhead |

⊘ The order matters and is not arbitrary: a trap profile taken under the raw client is a
different distribution from one taken under the LLM. `[measured w331]` `submit_ms` varied
**9.1x across three consecutive boots at one build** — so a regime that has not been measured
has not been characterised, whatever the other regime says.

★ **Parity will likely need a properly concurrent plane** (owner): one worker is enough to get
work off the vCPU, and is not enough to keep a real workload fed. That is the point at which
the thread model of §3 has to grow past one coordinator — and the wake design is already
chosen for it (an eventfd composes with the isolates' own descriptors in one epoll set; a
mutex does not).

## Testing, as a standing requirement (owner, 2026-09-12)

- **A bug worth finding is a bug worth a test.** Two of tonight's real defects were caught by
  tests that already existed and were simply not run (`an_armed_queue_head_write_records_and_returns`
  for the stale mirror, `a_power_on_reset_puts_the_emulated_gsp_back_to_cold` for the lost
  deferral arm). Two others have **no test and need one**: the vacuous working-set gate, and
  the silent write-back truncation that returns `NV_OK` with the guest's own request bytes in
  the tail.
- **Integration coverage wherever it does not need a GPU.** Most of the register model, the
  policy chain, the wire protocol and the address table are testable without hardware, and a
  test that needs a rented box is a test that runs once a day instead of every commit.
- **The mutation gate should be green with a good score.** A suite that passes while the
  mutants live is measuring its own existence, not the code — which is the same family as the
  green tests that held walls in place all through this campaign.
