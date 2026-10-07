# w247 — all THREE preconditions armed at once. The address plane is complete; nothing executes.

⊘⊘ **`CE-SUBMIT` is 0 and nothing executed.** No line here may be read as forwarded work.
Revision **`acbb9a3`**, stamped inside the binary. Control: `w246d_acbb9a3_witon_rbon`
(identical, `KAYFABE_FB_BACKING` unset).

## ⊘ The brief's question, refuted first

*"Does the semaphore's VA `0x2_0440fff0` resolve now that the witness is armed?"*
**It resolved in every boot this campaign has** — witness on and off:

| corner | witness | vidmem | `COMPLETION-DECLARE` | `COMPLETION-WATCH` | site |
|---|---|---|---|---|---|
| `w245off` | off | off | 8 | 8 | `GuestRam` |
| `w245on` | off | on | 8 | 8 | `GuestRam` |
| `w246c` | **on** | off | 8 | 8 | `GuestRam` |
| `w246d` | **on** | **on** | 8 | 8 | `GuestRam` |

All end `NOT-OBSERVED samples=88 … the address WAS readable and the declared payload never
appeared — a statement about the completion plane, not about the observer`.
⇒ **the witness is orthogonal**: it populates the CE channels' VAS (`pdb=0x201000`); the
semaphore resolves through the guest-RAM plane. `GR-ADDRESS-CENSUS` is likewise **invariant
across all four corners** (`operands=5 bound=4 unbound=1 mme_dwords=39`).

## The measurement — first boot with all three armed on a tree that boots

| | control `w246d` | **`w247` all three** |
|---|---|---|
| `GR-FB-BACKING` | 0 | **32** |
| `HostBackedFb` | 0 | **24** |
| `placed_as_asked=true` / `false` | 0 / 0 | **24 / 0** |
| `RING-VA-UNBOUND` / `PushbufferAperture` | 0 / 0 | 0 / 0 |
| `RingFbNeverWritten` | 0 | 0 |
| `GR-ADDRESS-CENSUS` | `5/4/1/39` | **`5/4/1/39`** |
| `SET_REPORT_SEMAPHORE` | `→ GuestRam` | **`→ GuestRam`** |
| **`CE-SUBMIT`** | **0** | **0** |
| `COMPLETION-WATCH NOT-OBSERVED` | 8 | **8** |
| `no-blocking-under-lock` | 0 | 0 |
| doorbells | 191/183/8 | **191/183/8** |
| `SMI_RC` / `CUP2_RC` | 0 / 124 | **0 / 124** |

**All five predictions, recorded before the boot, confirmed.** The crossing arms; the three FB
operands become real host `NV01_MEMORY_LOCAL_USER` objects mapped **FIXED** at the guest's own
VAs; the semaphore stays `GuestRam`; the census is unchanged because it counts **binding**, not
**backing**; and **nothing executes**.

## ★★★★★ The fifth instance — the crossing that matters was also behind an unarmed flag

`gr_execution_boundary.md` §4 records the emulated-framebuffer crossing as *"⊘ does not exist …
not built"*. **It was built the very next rung** (w228, `fb_leaf_crossing.md`) behind
`KAYFABE_FB_BACKING=on` — default off, for the now-familiar good reason. **`GR-FB-BACKING` is 0
in all nine of this campaign's boots.** The doc naming the dependency was never told the
dependency had been met.

## ★★★ The wall is a DELIBERATE REFUSAL, already argued

The GR doorbell is refused at `Route::NotACopyEngineChannel`. `gr_execution_boundary.md` (w227)
asked exactly this question and answered **NO**, naming four properties. The addressing half of
property 1 is now discharged. The two that remain are **not addressing**:

- **Property 2 — CLOSED**: the isolate's own ring/USERD/semaphore must be unreachable in the GR
  VAS. ⚠ A *subtraction*, and an architecture change. **This one leads.**
- **Property 3 — FAULTING/CONTAINED**: ⊘ `[NOT MEASURED]`. ★ `scripts/bench/gpu_fault_containment.sh`
  exists and has never been asked it — **the cheapest open item on the board, needing no new code.**

⊘ **Forbidden #1 holds**: nothing here writes the semaphore. The observer says so itself —
*"the observer WATCHES this address; it will never write it"*.

Full record: `docs/design/execution_plane_increments.md` §16.99.
