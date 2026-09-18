# The raw client passes on bare metal — so a guest failure indicts kayfabe

**STATUS: LIVE — measured 2026-09-18 (w756i), vast 51402274, RTX 3090, host driver
580.159.04, `kayfabe-rm-ladder --uvm-mean --mean-threads 8`. No guest, no QEMU, no KVM.**

```
P1 rm-invalidate  ✔ VERIFIED over 4 round(s)
P2 uvm-memop      ✔ VERIFIED over 4 round(s)
P3 rpc-bind       ✔ VERIFIED over 4 round(s)
STALE RACE        ✔ VERIFIED over 2 round(s)
THREADS           8 of 8 verified ✔    (28 overlapping pairs of graded intervals)
```

★★★ **Including `STALE RACE` and `THREADS`** — the exact two rows that fail under kayfabe.

## Why this measurement exists

> Owner, 2026-09-18: *"testing client to ensure it passes on bare metal before guest"*.

`bare_metal_pass_guest_fail_indicts_kayfabe` is a ruling this campaign already holds, but it
could not be **used** until the bare-metal half was measured: *"the client is probably fine"*
is a different claim from a run. With this, the guest's
`STALE RACE ★★★ CONTENT MISMATCH` and `THREADS 0 of 8` are attributable to kayfabe by rule
rather than by argument.

## What it isolates

The same client that remaps VAs correctly here reads **other lanes' patterns** in the guest.
The guest-side census says why in three numbers: **`maps=76  unmaps=0  replaced=0`** — nothing
in production ever removes a store slice, so mappings only accumulate and two VAs can come to
name one store offset.

⇒ The defect is in kayfabe's mapping **lifecycle**, not in the client and not in the diff:
`walkdiff::diff` computes the correct `Unmap`/`Remap` list and **nothing executes it**.

## ⊘ Scope, stated so it is not over-read

- **One run**, 4 rounds (the bare-metal default) against the guest harness's 8. It says the
  client is sound, not that it is sound under every schedule.
- ★ It is **non-vacuous where it matters most**: P3's negative control fired —
  `schedule REFUSED 0x40 BEFORE the promote` — so arm C's success is attributable to the RPC
  bind and to nothing else.
- ⊘ It says nothing about performance, only about correctness.
