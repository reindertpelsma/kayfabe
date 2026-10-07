# w288nc1 — CRITERION 1 IS **NOT MET**: the guest client's deliberate-fault probe could not be built

`[measured 2026-08-13, vh, real GA106 580.159.04, rev aea02a52]` — one boot, `--ce-client-fault`
in the guest via `KAYFABE_R33_ARGS`.

## What the guest client said, in its own words

```
info  R33 NOTIFIER APERTURE = SYSMEM (NV01_MEMORY_SYSTEM)
FAIL  R33 arm 4 = the probe could not be built: Other(2147483670)
      (an error here is never a fault — nothing had been submitted)
R33_RC=1
```

`2147483670` = `0x80000016`. ⇒ **The deliberate fault was never provoked in the guest**, so there
is no guest-side fault record to compare against the host's, and criterion 1 has no measurement.

⊘ **`MMU-FAULT-INFO relay lines` = 0** — and that is *correct*, not a relay defect: nothing asked,
because nothing faulted. `PLANE D UNMEASURED = 0` and `VA-IDENTITY BROKEN = 0` are likewise
**vacuous here** — they count guest-side reads that never happened. ⚠ Recorded explicitly so a
later reader does not mistake three zeros for three passes. *An absent artefact reads as
favourable.*

## What DID hold on this boot

- `ERROR-NOTIFIER built = 2, refused = 0` — the Tier-1 path ran for this client's channels.
- `NOTIFIER APERTURE = SYSMEM` — the aperture that actually exercises the rung, so the
  vidmem-`Unreachable` defect is **not** what silenced this arm. The client's own line states
  that hazard and it is not the one that fired.
- A host fault did occur: `Xid 31, ENGINE CE0 HUBCLIENT_CE1 faulted @ 0x1_20000000, FAULT_PTE,
  ACCESS_TYPE_VIRT_READ` — ⊘ but the client reported its probe unbuilt, so this cannot be
  attributed to arm 4 and must not be joined to it.

## Status of the three criteria

| # | criterion | result |
|---|---|---|
| 1 | guest observes the SAME fault by identity | ⊘ **NOT MET** — probe unbuilt, no guest-side record exists |
| 2 | negative control: no fault ⇒ nothing reported | ★ **MET** — native `Xid delta 95→95`; guest no-fault arm built 2 notifiers and reported nothing |
| 3 | cup2, `^CUP2_RC=` anchored | ★★★★★ **MET AND MOVED — 124 → 1** (see `traces/boots/w288nn/`) |

⇒ The rung's headline rests on criterion 3, which is the goal metric and is unambiguous.
Criterion 1 is **open**, and the next step is the allocation refusal `0x80000016` on the guest
probe — a client-side setup failure, upstream of anything this rung built.
