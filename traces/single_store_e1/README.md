# single-store increment 1 — the boot evidence

**Measured 2026-09-14**, vast instance `51038147` (RTX 3060 / GA106, driver **580.159.04**,
49 cores, 584 GB RAM), binary **`kayfabe-rev:3053761b34c80ef301de0ad3d589e25a9a12deb1`**,
which is also the tree revision — `BINARY_REV == TREE_REV` on every boot below.

⚠ **The revision is part of the citation.** Every row here is at that commit and nowhere else.

## The three arms, one variable

All three ran the standing "under the constraints" configuration (`KAYFABE_ISOLATES=real
KAYFABE_GUEST_RAM=memfd NVKVM_RAM_BACKEND=memfd KAYFABE_FB_JOIN=shared KAYFABE_GUEST_RING=ring
KAYFABE_GR_ROUTE=passthrough KAYFABE_CE_EXECUTOR=host`) and the raw-client mean hook at
`threads:8 rounds:8`. They differ in `KAYFABE_SCRATCHPAD` alone (and, for `e1req`, the probe's
starting point).

| tag | `KAYFABE_SCRATCHPAD` | `START_MB` | reservation | advertised FB | client |
|---|---|---|---|---|---|
| `e1ctl` | `off` | — | `DISARMED` | compiled 12288 MiB | **(P)** 8/8, `MEAN_FALSIFIER=PASS` |
| `e1full` | `on` | default 12288 | **`HELD` 11904 MiB** | **11904 MiB** (`fb_length=0x2e8000000`) | **(P)** 8/8, `MEAN_FALSIFIER=PASS` |
| `e1req` | `require` | 4096 | **`HELD` 4096 MiB** | **4096 MiB** (`fb_length=0x100000000`) | **(P)** 8/8, `MEAN_FALSIFIER=PASS` |

## ★★★ What each arm establishes, and what it does NOT

- **`e1ctl` is the claim no unit test can make**: with the gate off, this device is what
  shipped, *on real hardware, with a real guest*. Both census lines print
  `arm=off reservation=DISARMED`, which is what distinguishes the control arm from a binary
  that predates the arm.
- **`e1full` is the headline**: 11 904 MiB reserved as ONE non-contiguous host RM object, held
  for the life of the VM by an isolate that belongs to no guest process — and the raw client
  still passes. `nvidia-smi` read **11909 MiB used** while it ran.
- ★★★ **`e1req` is the falsifier for the derivation.** 11904 is close enough to the compiled
  12288 that a broken derivation could have looked right; 4096 could not. The advertised
  `fb_length` moves to `0x100000000`, so the size the guest is told really is the size the host
  gave us. ⊘ It also puts `enforce()` on the path: `require` admits a held reservation.

## ⊘ Three things these boots do NOT show

1. **`require`'s REFUSAL is untested live.** All three reservations succeeded, so the arm that
   refuses the device never fired on hardware. `enforce_arm` is unit-tested over all five
   outcomes (`crates/kayfabe-qemu-raw/src/scratchpad.rs`), and that is the whole of the
   evidence for it.
2. **`worst_trap` did NOT improve, and it was not supposed to.** 16.9 / 17.9 / 18.5 ms across
   the three arms — the per-proc lazy spawn still happens in every arm, because this increment
   *adds* a VM-lifetime isolate and deletes nothing (the sequencing rule). What is measured is
   that the new isolate's **`spawn_ms=1742.5`** is paid at PCI realize, where the guest does
   not exist — the same ~1.7 s that w470 caught a vCPU parked in, inside an MMIO exit.
   ⚠ Reading this as "the 1.6 s vCPU stall is fixed" would be wrong: it is one more spawn that
   never touches the guest's path, not the removal of the one that does.
3. **The client checks STATUSES, not content.** A `(P)` here is *"the registration path is
   served"*, not *"mappings are correct"* — the hook's own standing caveat.

## The numbers that are new

| | `e1full` (probe from 12288) | `e1req` (probe from 4096) |
|---|---|---|
| `spawn_ms` — fork + six namespaces + the blocking handshake | 1742.508 | 1727.905 |
| `probe_ms` — the halve-then-bisect reservation probe | **1273.747** | 15.515 |
| `reserve_ms` — the reservation itself | 14.561 | 3.428 |

★ The probe is the expensive half, not the reservation, and its cost is a function of how far
the bisection has to travel: from 12288 it is ~1.27 s of real multi-gigabyte alloc/free pairs;
from 4096 the first halving succeeds and the bracket is one grain wide. ⊘ Both are paid at
realize; neither is on any guest path.

`HOST_DMESG_XID=1` on all three arms, including the control — so the single host Xid is not
attributable to the reservation.

## Files

- `boot_e1{ctl,full,req}.log` — the harness's own output, including the pre-registered
  outcomes and every graded row.
- `run_e1*_dmesg.log` — the guest driver's own ring buffer (the serial log does not carry it).
- `run_e1*_hostdmesg.log` — the host driver's output for that boot.
- `scratchpad_census_lines.txt` — every `SCRATCHPAD` line from the three QEMU logs.

⊘ The QEMU logs themselves are ~7 MB each (one 120 KB census line per boot) and are not
committed; the lines that carry the result are extracted above.
