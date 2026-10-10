# Delegated decisions, 2026-10-10

**STATUS: LIVE, 2026-10-10.** The owner left the open decisions of the integration phase to the
coordinator ("you decide what is best so it is at least correct in all cases"). They are decided
here with the reasoning. The owner may overturn any of them; each says what that would cost.
The rule used for all of them: the STOCK install (no host kernel patch) must be correct in every
case; a patched host is an optional better tier (`design/V3_HOST_PATCH_LIST.md`).

| # | Question | Decision | Why | Status |
|---|---|---|---|---|
| D1 | A split big leaf outside reservations: 4 KiB grain or a micro reservation per row | Both, in this order: a micro reservation sized to the row when host RM accepts it; otherwise 4 KiB grain (one host mapping per 4 KiB page). Never re-make a kept mapping. | The owner invariant is that an unchanged VA is never unmapped during a refresh. 4 KiB grain depends on no RM partial-unmap semantics, so it is the always-correct floor; the reservation keeps big PTEs and costs 2 calls per row. | implemented on `claude/batched-map-decisions-20261010` (see its report) |
| D2 | Act-thread ledger locks vs the no-stall rule | Accept, on conditions: no ledger lock is held across a host call, a syscall, logging or an allocation proportional to n; critical sections are bounded and chunked; a test asserts the bound. | The alternative, a steer request answered by the VA thread, makes the act thread wait on the VA thread, which is strictly worse. | same branch |
| D3 | `KF3_BATCH_MICRO_RESERVE` default | On. A refused reservation falls back to D1's 4 KiB floor. The reserve probe becomes a gate. | Without it a row above 2^20 pieces is refused by name (a correctness hole). The probe passed at 6fafcc6e (measured); the unexplained `nv01-control` mismatch concerns the NV01 path we avoid, so it does not block this. Re-measure on `459da55d` and on every driver version. | same branch; gate not yet run |
| D4 | Which class-B behaviour flags to hardwire next | All class-B flags that the production profile needs, in the inventory §9 order, but only after the TDR fix lands (it may change the set) and the Linux regression passes. Flags that carry a captured table (`KF3_DISPLAY_CAPS_PROBE`, `KF3_GFX_POOL_PROBE`, the `KF3_DISPLAY_CTRL_PROBE` echo) are re-derived from ogkm / host controls first; they are never hardwired as written. Windows-only behaviour is keyed on the guest's own declared state, not a flag. | `V3_FLAG_INVENTORY.md` rules + derive-never-capture. | not started |
| D5 | Fixed broker (`/opt/nvkvm-broker-next`, 9b5f64b) as the `interactive.sh` default | Yes, after `broker_lane.sh` passes and the owner has had the Linux desktop on it. | It carries bug fixes (branch `kf-broker-fixes-20261008`); the old broker stays reachable by an explicit path. | waiting for the Linux retest |
| D6 | `claude/vfio-refusal-ablation-20261009` | Stays unmerged (diagnostic only; conflicts on `tools/vfio-gsp-observer`). The branch is kept as the archive. | Merging adds a conflicting old tool to the product tree for no product value. | closed |

## Standing follow-ups the owner asked for

- **Host kernel patch tier**: the optional enhancements (exact partial unmap, in-kernel doorbell,
  USERD-in-sysmem fix, uvm managed memory, mdev shim) are to be implemented as a host kernel module
  patch. Plan: `design/V3_HOST_PATCH_LIST.md`.
- **Windows app matrix**: the app matrix must eventually also run on Windows guests. A Windows lane
  needs a working Windows run first (the TDR hunt, branch `claude/tdr-hunt-20261010`), an app
  inventory, and the scripted sign-in harness brought into git.
- **Models**: `OWNER_RULINGS.md` §AB.
