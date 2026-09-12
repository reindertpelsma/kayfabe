# Resume here — w530, 2026-09-12

**STATUS: LIVE.** Supersedes `RESUME_HERE_w494.md` for everything about traps and locks.

## Where the two goals stand

**1. Raw client — DONE, confirmed n=2 (w525, w530).**

```
W392D_GUEST_OUTCOME=(P)   MEAN_FALSIFIER=PASS   THREADS 8 of 8   panics=0
VCPU-BLOCKING none — no blocking door was reached on a vCPU thread
inline_exceptions=0   worst_trap 2.8–3.6 ms   slow_waits=0 on EVERY lock rank
```

From `worst_trap 1.81 s` / `6776` slow traps / `inline_exceptions=166` at w394. The owner's
first invariant now holds **by measurement**.

**2. LLM — blocked, and NOT on traps.** `W392_OUTCOME=(Z)`, zero tokens, while the same boot's
CPU oracle produces 16. On the trap axis the lane is already clean (`VCPU-BLOCKING none`,
`inline_exceptions=0`, worst trap ≈4.5 ms). ⚠ Do **not** report it as "slow-trap free": a
clean census over a workload that never ran is `every_row_verified_over_zero_rows`.

## What the LLM lane is actually blocked on

```
MINMM_EXC: CUDA error: CUDA-capable device(s) is/are busy or unavailable
```

A **4×4 matmul** — before any model, any weights, any memory pressure. Two `Xid 31` per python
process, the same pair every time:

```
ENGINE GRAPHICS HUBCLIENT_FE    faulted @ 0x74xx_xxxxxxxx  FAULT_PDE VIRT_WRITE
ENGINE CE2_PBDMA0 HUBCLIENT_ESC faulted @ 0x2_00218000     FAULT_PDE VIRT_READ
```

⊘⊘⊘ **THE REFUSALS ARE NOT THE BLOCKER — corrected.** The guest log is full of our own
`status=0x56` (`NV_ERR_NOT_SUPPORTED`) refusals, and I first reported them as the cause. They
are the **modprobe-time baseline**: `a_refusal_needs_a_negative_control` ran this control at
w337 and tabulated `hClass 0x70 0x208f 0x402c 0xc36f` and `user_shared_data ×5` as
**identical** in boots where `cup8` is bit-exact green. ⇒ **A `0x56` in a failing run is
evidence of nothing until it is checked absent from a passing one.**

★★★ **THE REAL LEAD: the configuration that PASSED is the one that now cannot boot.** w383's
passing LLM ran with `KAYFABE_DOORBELL_ASYNC=off`, and today that arm fails at
**`RmInitAdapter`** — *before any doorbell is rung*, which a doorbell-arm flag should not be
able to reach. That anomaly has a **seconds-long repro** (boot, modprobe, look for adapter
output — no CUDA, no model, no 20-minute run) and is bisectable, where the LLM is not.

## Two dead ends — do not re-run them

- ⊘ **The w383 coalescing explanation is REFUTED on this tree.** Its signature matched exactly
  (*"two `Xid 31` per python process"*) and its recorded fix does not work: `nocoalesce` fails
  identically to `on`. That conclusion tracked a version, not a mechanism.
- ⊘ **The `off` arm no longer boots** — *"NOTHING from RmInitAdapter, and nvidia-smi did not
  succeed either"*. ⚠ Do not file this as "a stale arm, low priority": it is the arm the LLM
  **passed** on at w383, so it is the shortest route back to a known-good baseline, and its
  failure point (adapter init, before any doorbell) says the flag is reaching something it
  should not.

## Still open

- **Needs the owner:** may a page-table sweep commit land in **chunks**? It holds the Device
  and Proc ranks ~5 ms, ~1066 times a boot. Nothing waits on them now, so it is no longer
  urgent. ⊘ `PublishedUnbind::Refuse` does **not** make chunking safe — it protects only
  host-published rows, so a doorbell between batches could still see a partial commit.
- **Delete before shipping:** `KAYFABE_TRAP_FATAL_US`, `KAYFABE_STALL_ALARM_US` (+ `_AT`,
  `_WORKER`), `TRAP-CPU`. ⚠ `KAYFABE_STALL_ALARM_AT=<off>` is the tool that named the last two
  stalls; do not delete it before the LLM lane is done with it.
- **Orphans:** the `DoorbellTable` (316 lines, 9 green tests, zero users) and the fn-71
  `Reassembler`. ⊘ The table's stated purpose — take the reach away from the trap — is now met
  *by measurement* (`VCPU-BLOCKING none`); wiring it would make that **structural** rather than
  disciplinary. Still worth doing, no longer urgent.
- **The vacuous #14 working-set gate.**
- ⚠ **One instrument disagreement:** `TRAP-CPU` is armed only in the **write** entry point,
  `TRAPWITNESS` covers reads too. The remaining slow trap is therefore a **read** of
  `bar0+0x110094`. Either arm `TRAP-CPU` on reads or stop comparing the two numbers.
