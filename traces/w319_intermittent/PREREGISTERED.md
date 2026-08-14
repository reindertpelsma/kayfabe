# w319 — PRE-REGISTRATION, written BEFORE any boot of this rung

> **STATUS: LIVE — 2026-08-14 (w319).** Branch `w319-intermittent-fault-pde`, base master
> `ef05f9b3`. Bench **`vh`**, real GA106, host driver open `580.159.04`.
> ⚠ Every prediction below is committed **before** the arms run. A knob that "makes it worse"
> is only evidence if the direction was named first.

---

## 0. ★★★★★ THE MECHANISM IS ALREADY ATTRIBUTED — FROM w314's OWN COMMITTED LOGS, ZERO BOOTS

This was not found by sampling a 20 % event. It was found by reading the artefacts w314
already committed, and it is unambiguous.

`[measured w319, 2026-08-14, from `traces/w314_confirm/run_*_qemu.log.gz`]`

| boot | `CUP3_VAL` | `pinned/asked` | `DRAIN_MS` | `last_pinned_va` | wall budget |
|---|---|---|---|---|---|
| `w314basecup3` (clean master) | **RED** | 11883 / 13313 | 3000 | `0x2_0326a000` | ⚠⚠ **HIT** |
| `w314cup3` (branch) | **RED** | 11810 / 13313 | 3000 | `0x2_03221000` | ⚠⚠ **HIT** |
| `w314br1` | 43 | 13313 / 13313 | 2672 | `0x2_047ff000` | no |
| `w314br4` | 43 | 13312 / 13313 | 3000 | `0x2_047fe000` | ⚠⚠ hit, **1 row short** |
| `w314bt1` | 43 | 13313 / 13313 | 2898 | `0x2_047ff000` | no |

**The faulting page `0x2_0440f000` lies strictly between where the reds stopped and where the
greens reached.** `0x2_03221000 < 0x2_0440f000 < 0x2_047fe000`.

⇒ **THE MECHANISM.** `SharedDoorbell::publish_vas_rows` drains the doorbelled VAS's guest-RAM
rows in ascending-VA order inside the doorbell trap, bounded by
`VAS_DRAIN_WALL_BUDGET = 3000 ms` (`crates/kayfabe-qemu-raw/src/shim.rs:13405`). The drain's
own cost is **13 313 rows × 199–280 µs = 2.65–3.73 s**, which **straddles its own 3 s budget**.
When the per-row cost runs high the loop `break`s early and every row after that point is
**never attempted** — including the completion-semaphore page `0x2_0440f000` that
`completion_watch.rs:703-705` names. The engine then writes it and the host MMU reports
`FAULT_PDE` because no directory was ever installed for it.

★ **It is not a thread race.** Publication is synchronous on the vCPU thread inside the MMIO
trap, strictly before the forward (`shim.rs:4826` → `shim.rs:4896`). ⇒ **the brief's
publication-ORDERING-race hypothesis is REFUTED as stated.** The defect is a **budget
truncation**, not an ordering race — same family, different mechanism, and the difference
decides the fix.

★ **The brief's "oddity" needs no explanation.** CE3 / `HUBCLIENT_CE1` writing what the brief
calls a "GR" page is expected: `0x2_0440f000` holds the guest's eight `SET_REPORT_SEMAPHORE`
targets `0x2_0440ff80 … 0x2_0440fff0`, which are written by whichever engine executes the
release. `completion_watch.rs:703` recorded this exact `CE3 HUBCLIENT_CE1` attribution at w265.

★ **And the regression that made it possible is named.** `SharedDoorbell::pin_completion_guest_ram`
— an **unconditional** pin of exactly this page, which `shim.rs:3851` records took these Xids
to **zero** at w266 — was **deleted at w304** (`f20ab952`) on a "strict superset" argument.
The candidate *set* is indeed a superset; the *delivery* is not, because the drain is
budget-truncated. **An unconditional pin was replaced by a conditional one.**

---

## 1. The instrument (added this rung, default-off, master-identical when unset)

Two env knobs on the drain, both `#[cfg(feature = "host-isolates")]`, both announced in the
boot's own `VAS-PUBLISH` line as `W319KNOB[budget_ms=… row_limit=…]` on **every** boot
including the default one — so `absent` reads as *old binary*, never as `3000`.

- `KAYFABE_VAS_DRAIN_BUDGET_MS` — overrides `VAS_DRAIN_WALL_BUDGET`. The **faithful** knob: it
  is the same clock the defect actually trips on. Absent ⇒ 3000 ms.
- `KAYFABE_VAS_DRAIN_ROW_LIMIT` — caps the drain at a **row count** instead. The
  **deterministic** knob: a clock truncates at a different row every boot, a count does not.
  Absent ⇒ `VAS_DRAIN_ROW_CAP` = 65 536.

⊘ Neither is a fix, neither is on by default, and neither changes a single byte of behaviour
when unset.

---

## 2. ★★★ THE ARMS AND THEIR PREDICTIONS — committed before the first boot

### Arm **R** — REPRODUCE ON DEMAND (deterministic). `KAYFABE_VAS_DRAIN_ROW_LIMIT=11800`, n=3

11 800 sits just **below** both reds' row counts (11 810 and 11 883) and far below the greens'
13 313, so the drain is forced to stop in the same region the reds stopped in.

**PREDICTED:**
1. **3 / 3 RED** — `^CUP3_VAL` != 43 on every boot. ⊘ Under the baseline ~20 % red rate, 3/3
   red has p ≈ 0.008.
2. Host `dmesg` non-empty on every boot, carrying `Xid … 31 … FAULT_PDE ACCESS_TYPE_VIRT_WRITE`.
3. Fingerprint predicted **`ENGINE CE3 HUBCLIENT_CE1 faulted @ 0x2_0440f000`**.
   ⚠ A fault at a **lower** VA is also consistent with the mechanism (the engine may touch an
   earlier unpublished page first) and will be reported as a partial match, not as a pass.
4. QEMU log carries `⚠⚠ DRAIN ROW CAP 11800 HIT`, `complete=false`, `pinned=11800`.

**FALSIFIER:** any boot returning `43` ⇒ the truncation point is **not** what decides it, and
this whole attribution is wrong. Report immediately.

### Arm **M** — MODULATE WITH THE FAITHFUL KNOB (the clock). `KAYFABE_VAS_DRAIN_BUDGET_MS=2500`, n=2

**PREDICTED:** **2 / 2 RED**, `⚠⚠ WALL BUDGET HIT`, `complete=false`, `DRAIN_MS=2500`,
`pinned` in roughly 9 000–11 500. This arm tests the mechanism itself rather than a proxy for
it. **FALSIFIER:** a green boot with `budget_hit` and a truncation below `0x2_0440f000`.

### Arm **H** — DRIVE THE RATE DOWN. `KAYFABE_VAS_DRAIN_BUDGET_MS=20000`, n=4

**PREDICTED:**
1. **4 / 4 GREEN** — `^CUP3_VAL=43`.
   ⊘⊘ **STATED PLAINLY BECAUSE IT MATTERS: 4/4 green is NOT by itself discriminating.** Under
   the baseline ~20 % red rate, 4/4 green happens with p ≈ 0.41. It is the weakest clause here
   and must not be read as the result.
2. ★ **THE DISCRIMINATING CLAUSE — `budget_hit` absent on 4/4, `complete=true` on 4/4,
   `pinned=13313/13313` on 4/4.** At the default budget, w314's five recorded boots carry
   `budget_hit` on **3 of 5**; 0/4 under this arm has p ≈ 0.026 under that rate. **This is the
   clause the arm is graded on**, because it is the mechanism variable rather than its
   downstream consequence.
3. `DRAIN_MS` reported per boot — the honest cost of the candidate fix, for clause (b).

**FALSIFIER:** a red boot with `complete=true` ⇒ a complete drain is not sufficient, and there
is a second mechanism.

---

## 3. ⊘ What this rung does NOT claim

- **No baseline re-measurement.** w314's n=10 (4/5 branch, 4/5 same-hour master) is the control
  and is not re-spent. This rung's arms are graded against it, not against each other alone.
- **A candidate fix is measured, not shipped.** Arm H raises a budget that is held under the
  QEMU BQL with every vCPU halted, and w314 measured the surrounding disposal already at
  2.65–2.92 s of a 4 s `scrubberDestruct` budget. **Raising this budget spends headroom that
  is already 73 % consumed**, so it is reported as a measurement and an owner decision, never
  as a merged fix.
- **`n` is small on every arm.** The strength here is the *direction of the modulation* and the
  *per-boot mechanism variable*, not the sample size of the binary outcome.
