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

The guest kernel log is full of **our own refusals**, `status=0x56` (`NV_ERR_NOT_SUPPORTED`),
on controls CUDA needs and the raw client never asks for —
`INTERNAL_INIT_USER_SHARED_DATA`, `INTERNAL_USER_SHARED_DATA_SET_DATA_POLL`,
`INTERNAL_CE_GET_PCE_CONFIG_FOR_LCE_TYPE` — plus a failed `GspRmAlloc` of `hClass=0x70`.

⇒ **This is the cudart-controls correctness campaign** (`the_llm_wall_is_four_cudart_only_controls`,
`a_refusal_needs_a_negative_control`), not a trap or lock one.

## Two dead ends — do not re-run them

- ⊘ **The w383 coalescing explanation is REFUTED on this tree.** Its signature matched exactly
  (*"two `Xid 31` per python process"*) and its recorded fix does not work: `nocoalesce` fails
  identically to `on`. That conclusion tracked a version, not a mechanism.
- ⊘ **The `off` control is UNUSABLE.** With `KAYFABE_DOORBELL_ASYNC=off` the guest driver never
  initialises — *"NOTHING from RmInitAdapter, and nvidia-smi did not succeed either"*. The path
  bit-rotted after `on` became the default at w467. **The three-arm comparison the w383 record
  rests on cannot be reproduced**, so the doorbell arm can be neither blamed nor cleared by it.
  ⚠ If the `off` arm is meant to stay supported, that is a separate bug worth its own fix.

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
