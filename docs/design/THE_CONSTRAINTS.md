# THE CONSTRAINTS — what kayfabe must do, not just do

**STATUS: LIVE, 2026-09-13.** Owner's list, given in conversation this date and consolidated here
so it survives a context compaction. ⊘ This supersedes nothing; it COLLECTS what is scattered
across `THE_OVERNIGHT_DIRECTIVE.md`, the design docs and agent memory.

> **Why this file exists.** The owner, 2026-09-13: *"we already got the LLM passing under
> everything trapped on kayfabe a few days ago… If bar1/bar2 is now STILL trapped under your cuda
> runs then its worthless to continue, you are basically doing something that worked. The entire
> reason I am pursuing this is because I want it to get to work under the constraints."*
>
> ⇒ **Functionality is not the deliverable. Functionality UNDER THESE CONSTRAINTS is.** A green
> workload that meets none of them is a repeat of work already finished in early September.

## The list

1. **No traps in BAR1, BAR2 or PRAMIN.**
2. **Only WRITE traps in BAR0**, PRAMIN excepted. (Read traps in BAR0 are already shown
   unnecessary.)
3. **Multiple concurrent workers in isolates, with multiple parallel transactions.**
4. **All traps sub-millisecond.** The PRAMIN base re-point may be longer — it is the one
   sanctioned expensive trap (`the_write_trap_contract.md`).
5. **The new DoorbellTable wired.**
6. **Every MMIO trap only posts the register write to a queue** (optionally clearing another
   register to close a race), wakes a worker, and returns.
7. **epoll in workers**, so more CUDA work runs in parallel than there are workers.
8. **Workers do all the work**, and completion writes land in VMM memory, asynchronously from the
   vCPU threads.
9. **Emulated channels actually use the scratchpad** to do work when work is needed.
   ⊘ Not forged: *"a scrub must be executed, on scratchpad, if its from an emulated channel."*
10. **Scratchpad work can go from polling to waiting on an eventfd** to cut CPU load (sets the
    eventfd IRQ).
11. **Interrupt forwarding actually works** — passthrough when libcuda falls back from semaphore
    to eventfd, and the emulated-channel wake-up IRQ too (`the_interrupt_arming_model.md`).
12. **No hardcoded single chip.** Any die must work, as in `nvkvm-pv`. A per-die fact must be one
    of: derived from ogkm source · obtained by an **unprivileged** host userspace ioctl ·
    computed · a stub that satisfies ogkm because guest userspace does not care · or defined per
    ARCHITECTURE FAMILY so it stays maintainable.
13. **No raw VMM pointers in safe code.** They belong in `unsafe` only, and safe code is always
    bounds-checked rather than trusted to have been written correctly.
14. **Isolates can have multiple threads** executing several CUDA operations in parallel, as
    `nvkvm-pv` does.
15. **The two vidmem worlds are disjoint** — see below.
16. **Host userspace stays UNPRIVILEGED.** Standing, absolute, and it constrains every item above.

## 15, in full — the split that is easy to get wrong

> Owner: *"the clean split that pramin/bar2 is from fake fb and that bar1 is only mapping from the
> real guest vidmem… we discovered that **all fake fbs were only used in bar2 and pramin and not
> bar1/userspace channels, and none of the real userspace vidmem were used in bar2/pramin but only
> bar1/userspace channels**"*

| world | backing | serves |
|---|---|---|
| the reserved object | **ONE** device-local RM object, all guest vidmem | BAR1, userspace channels, engines |
| the aperture store | **ONE sparse memfd**, same advertised VRAM size, only touched pages resident (a few MiB) | BAR2, PRAMIN |

- Guest vidmem is **one reserved RM object**; channels map **slices** of it, chosen by the
  guest's own **PD*/PT*** entries.
- Those tables are **published and updated ONLY in the refresh function** — not on demand, not at
  a trap.
- The disjointness is an **empirical finding**, not an aspiration: it is what allowed the aperture
  backing to be stripped of GPGA positions entirely.

### ⊘ Naming — why not "GPGA real" / "GPGA control"

The owner's working terms are *GPGA real* and *GPGA control*. `control` is **already three things**
in this tree — the control **plane**, an experiment **control arm** (`KAYFABE_PREMAP_BAR1=0`), and
RM **control** commands (`NV2080_CTRL_*`) — so `GpgaControl` reads as at least two wrong things at
every call site. Proposed instead, and the split is then legible from the name alone:

- **`GpgaDevice`** — the reserved device-local object. What an ENGINE reads and writes.
- **`GpgaAperture`** — the sparse memfd. What the guest CPU pokes through an aperture, and
  nothing else. (`pramin_is_a_bringup_aperture_not_a_running_path` already calls PRAMIN exactly
  that.)

⊘ Avoid `GpgaShadow`: `gpga_is_one_reserved_object.md` uses "shadow" pejoratively for the
`install_join` mechanism it deletes, so the word already means "the thing that was wrong".

## Measured state, 2026-09-13 (rev `d8bc93bf`)

| constraint | state | evidence |
|---|---|---|
| 1 — no BAR1/BAR2/PRAMIN traps | **HOLDS**, both workloads | `TRAP_FILLS=0`; every fill was a premap install (w696: `1728+211 == premap 1939`; `5841+294 == premap 6135`) |
| 2 — BAR0 write-only | HOLDS except the counter page | ~132 reads at `+0xbb0000`; the device-view wire verb is unbuilt |
| 5 — DoorbellTable | wired | goal 4, w656–w660 |
| 15 — disjoint worlds | **NOT HONOURED** | one `BarMirror` + one store serves BAR1 **and** BAR2; the real-object join is per-LEAF, not per-BAR (`barmirror.rs:19-21`) |
| 15 — one reserved object | **designed, not wired** | `gpga_is_one_reserved_object.md` is STATUS: LIVE and says the per-leaf join *"is scheduled for deletion by it"*; `reserve_gpga` has no caller outside its own crate; no GPGA line in any boot |
| 4, 8 — sub-ms / off-vCPU | **HOLDS**, with the sanctioned PRAMIN exception | `VCPU-BLOCKING total=22 doors=3` and `PRAMIN-SLOT moves=22` — **the 22 doors ARE the 22 PRAMIN re-points**. `moves=22` is IDENTICAL under the raw client and CUDA (only `skipped` moves: 18439 vs 5448), so it is a BOOT-TIME set and the owner's ruling (*"297us for a thing that only happens at boot… thats fine for that mmap"*, `move_ns[worst=296558 mean=74698]`) is not expired |
| 7, 14 — epoll / threaded isolates | not built | |
| 12 — any die | not done | ~20 GA106 `ChipProfile` fields are per-die measurements |

## ★ Constraints 12 and 15 share ONE prerequisite (found 2026-09-13, w696d)

`ChipProfile` is a compile-time **`static`** (`ga10x.rs:1686`, `pub static GA106: ChipProfile`),
and the WPR2 addresses inside it are computed by `const fn` from a hardcoded `FB_SIZE_MB = 12288`
(`gsp_fw_wpr_end()`, `ga10x.rs:224`).

⇒ **Constraint 15** says *"the advertised framebuffer size is derived from the reservation that
succeeded, never asserted ahead of it"* — impossible against a `const fn`.
⇒ **Constraint 12** says no per-die constants — and `FB_SIZE_MB` is one, as are ~20 other
`ChipProfile` fields.

**Both need the same thing: `ChipProfile` constructed at START, not at compile time.** They are
one refactor, not two.

⚠ And the advertised number is already wrong by 2x: `[measured 2026-09-11, bare metal]` the
largest single vidmem reservation is **6144 MiB against 12288 advertised**. So today the guest is
told it has twice the video memory we can actually reserve for it.

### Proposed order (each rung independently green-able)

1. `ChipProfile` becomes a runtime-constructed value; `GA106` becomes a constructor call with
   today's constants as inputs. ⊘ **No behaviour change** — the same bytes, computed later. This
   is the load-bearing rung and the only risky one.
2. Reserve ONE object at start; refuse to boot if it fails; derive the advertised FB size from
   what succeeded and feed it into (1).
3. BAR1 served as SLICES of that object, published in the refresh function only.
4. BAR2/PRAMIN moved onto `GpgaAperture`; the two worlds disjoint BY TYPE.
5. Delete `install_join` and the per-leaf machinery.

⊘ Rung 1 is where constraint 12 starts too — the per-family derivation has somewhere to live only
once the profile is a value.

## ⊘ Three false violations in one session, all caught by opening the counter

Recorded because the pattern is the point, not the individual errors:

1. **BAR1/BAR2 "1728 traps"** — `fills` summed premap installs; `TRAP_FILLS` was 0.
2. **BAR1/BAR2 "5841 traps" on the raw client** — same counter, same error, second workload.
3. **"22 vCPU blocking doors"** — they are the 22 PRAMIN re-points, already measured at 297 µs
   worst and already ruled sufficient by the owner **on this same date**, with the ruling's expiry
   condition written down and NOT met.

★ Each one read as a regression of a goal the directive lists as complete. ⇒ **A violation claim
is a decision input and earns the same scrutiny as a green.** Two of the three had their answer
sitting in a doc comment or a dated ruling in the same file as the counter.

## ⚠ The instrument warning that governs this file

`[measured w696]` The BAR-mirror fill counter summed premap installs into a number whose own
docstring said *"every fill was ONE trapped access"*. It read as thousands of traps when the trap
count was **zero**, and it was reported as a goal-2 regression twice — the second time changing a
project decision. ⇒ **Before reporting any constraint as violated, open the line that CHANGES the
counter.** A name and a docstring are not substitutes, and this class has now cost w607, w627,
w695l and w696.
