> ⊘⊘⊘ **ARCHIVED — THIS DESCRIBES A SUPERSEDED ARCHITECTURE. IT IS REFERENCE, NOT CURRENT.**
> The live design is `docs/design/THE_DESIGN.md`. This file is kept because its *measurements*,
> its *ogkm findings* and its *reasoning* remain useful — its **architecture does not**. Do not
> implement from it, and do not cite it as current. Archived 2026-09-21 (w822).

# The adversarial guest — kayfabe's first deliberately hostile client

**STATUS: LIVE, 2026-09-19 (w749). Compiles clean against 6.8.0-59-generic on `vh3`
(`make RC=0`, `advguest.ko` produced, `modinfo` OK). NOT yet run against a live
emulated GPU — see §6 for what is verified vs. what is built.**

> **Owner:** *"We build our own mini ogkm kernel module, a test suite model. It does
> things that violate in ogkm … Its basically a client to kayfabe that kayfabe
> considers adversary."* · *"the boot sequence can be imported or executed"* · *"the
> adversarial module may import any ogkm code … ogkm is OSS."*

## 1. What it is

`scripts/advguest/` builds **`advguest.ko`**, a Linux kernel module that pretends to
be a **hostile NVIDIA kernel driver**. It maps the kayfabe emulated GPU's
guest-visible MMIO surface **directly** — BAR0 registers, the doorbell page, the GSP
command queue, the MMU-invalidate registers, the BAR1/BAR2 windows — and never
touches `/dev/nvidia*`. From there it fires a suite of adversarial submissions and,
after each one, checks that the VMM is still serving.

It boots with **no real disk**: an initrd containing busybox and the module, loaded
as a single `insmod` from `/init`. The suite runs in the module's `module_init`,
prints one self-describing line per case to the kernel log (serial console), ends
with a tally line, and powers the guest off. `advguest_init` returns `-ENODEV` on
purpose so the one-shot suite unwinds cleanly and can be re-run with another `insmod`
— all BAR mappings and the `pci_dev` reference are dropped in `adv_teardown()` first.

It reuses the vendor's own open-driver headers as an oracle (owner addendum #1):
GPFIFO/USERD layout (`clc36f.h`, `ga100/dev_ram.h`), the B0B5 copy-engine method
stream (`cla0b5.h`), the VF register offsets (`turing/tu102/dev_vm.h`). Every number
in `advguest.h` carries an `ogkm-580:` or kayfabe-source citation. This lets it emit
**structurally valid** submissions that are **semantically hostile** — a well-formed
CE pushbuffer naming an address it must not reach — which is far harder to refuse
correctly than a garbage dword.

## 2. The property under test

> **kayfabe must REFUSE BY NAME and STAY ALIVE.**

The suite PASSES when **every** adversarial input is refused or contained **and** the
VMM (the kayfabe archive linked into `qemu-system-x86_64`) is still serving MMIO at
the end. A VMM crash, a hang, or a **silent accept** of a dangerous operation is a
FAIL. This is stated verbatim in the module's header comment.

This is the value proposition made executable: `hostile_guest_isolation_is_the_value
_proposition`. A guest that controls its kernel driver is exactly the threat model,
and §45 of `THE_CONSTRAINTS.md` draws the line this suite probes — kayfabe runs as
host userspace, so it *cannot* supply a privileged value (the ceiling is structural),
but a guest-chosen number used as an **address into an object we hold** is ours to
refuse, and that is what most of phase 1 attacks.

## 3. The four phases (owner addendum #2)

An adversary that attacks from a cold, hand-rolled state mostly tests early refusals.
One that attacks from a **correctly initialised** state tests the paths that only
exist once WPR2 is up, channels are scheduled and VA spaces are live — where this
project's real defects have been found. So bring-up is a phase, and its success is
its **own** reported verdict.

| phase | what | verdict on failure |
|---|---|---|
| **0** | discover the device, map BARs, assert a known-good state; optionally load the real nvidia stack first (ogkm's boot sequence executed) | its own PASS/FAIL; if it fails, **phases 1-3 are NOTRUN** |
| **1** | well-formed but hostile submissions — the valuable ones | REFUSED (contained + alive) / FAIL |
| **2** | malformed / fuzzed inputs and raw register abuse | REFUSED / FAIL |
| **3** | concurrency — kthreads hammering doorbell + invalidate + queue heads | REFUSED / FAIL |

⚠ **NOTRUN is not PASS.** If phase 0 does not reach a known-good state, every later
case is reported NOTRUN — unmeasured, never green. This is
`a_census_zero_needs_a_known_positive` built into the structure: the suite whose
positive control never ran must not read as passing (the trap that reported eight
`--alias-*` arms as failures that had never asked their question).

### `post_init` — cold vs. post-init reach

`build_adv_guest.sh ADV_POST_INIT=1` stages the host's real `nvidia*.ko` + GSP
firmware into the initrd, and `/init` loads them **before** `insmod advguest.ko`.
That runs ogkm's genuine devinit / GSP bring-up, so the module then attacks a
post-WPR2, channels-scheduled GPU. The module reports the reach it attacked from as
`P04 post_init_state` and threads it into every phase-1 `want=` field: post-init, a
hostile submission can reach the deep pushbuffer decoder and should provoke a named
`FwdFault::…`; cold, the same submission is a `dbtable::Route::Unallocated`
non-event, and the module says so rather than claiming a decoder refusal it could not
have caused.

## 4. The cases (~25)

Phase 0 — `P00` device_present · `P01` bar0_mappable · `P02` bars_mappable · `P03`
identity_readback (liveness baseline) · `P04` post_init_state.

Phase 1 (well-formed hostile) — `A10` CE `LAUNCH_DMA` `SRC_TYPE=PHYSICAL` naming an
address outside the FB · `A11` CE operand straddling the end of the store · `A12` CE
operand in an aperture we do not model (reserved PHYS target) · `A13` CE semaphore
release to an unmapped VA · `A14` GPFIFO entry with `LENGTH=0` · `A15` GPFIFO entry
with absurd `LENGTH` and a bogus pushbuffer VA · `A16` USERD `GP_PUT` beyond the
ring, `GP_GET` into non-RAM.

Phase 2 (malformed / register abuse) — `A20` doorbell token out of range · `A21`
doorbell token for a channel never created · `A22` doorbell token with bits above the
11:0 vector field · `A23` GSP queue-heads written with garbage · `A24` MMU invalidate
with PDB 0 · `A25` MMU invalidate naming a non-existent PDB · `A26` `ALL_PDB` flood ·
`A27` MMU invalidate trigger with no preceding PDB write · `A28` write to a dead BAR0
offset · `A29` read near the end of the BAR0 aperture · `A30` BAR1 write to an
unmapped VA · `A31` BAR2 write to an unmapped VA · `A32` pseudo-random fuzz across the
known BAR0 registers.

Phase 3 (concurrency) — `A40` doorbell storm from N kthreads · `A41`
invalidate+doorbell race · `A42` all GSP queue-heads written concurrently.

Each line looks like:

```
ADVGUEST A20 doorbell_token_out_of_range = REFUSED (expected) want=Route::Unallocated (non-event)
ADVGUEST A10 ce_launch_phys_outside_fb = REFUSED (expected) want=FwdFault::PushbufferAperture|CpuCeFb
```

and the suite ends with a tally written by the code that owns the counters:

```
ADVGUEST_TOTAL pass=N refused=N fail=N notrun=N alive=1
ADVGUEST_VERDICT=PASS (every adversarial input contained; VMM still serving)
```

## 5. What the guest can and cannot see — the two-sided verdict

The module runs **inside** the guest, so its only in-guest observable is *"did the
device still respond sanely after the op?"* — `adv_alive()` reads `TIME_0`
(`0x00BB0080`), which reads all-ones from a dead MMIO region and a running nanosecond
clock from a live one. That is the **liveness/containment** half.

kayfabe's refusal **by name** — `FaultTag("FwdFault::PushbufferAperture")`, the
`DOORBELL … REFUSED [kind]` line, the teardown census — is printed **host-side**, into
the qemu log, and never into guest memory (doorbell writes are fire-and-forget; the
GPFIFO contract reads nothing back). So `run_adv_guest.sh` greps the qemu log for
those names as a **coverage report**, and each case advertises the name it targets in
`want=`. Neither half alone is the whole property; both are reported.

⊘ Two honest limits, both by design in kayfabe:
- A true `Route::Unallocated` doorbell is a **non-event with no log line** (dbtable
  is lock-free and silent for it), so its by-name check is "contained + alive", not a
  grep hit. The module labels these `(non-event)`.
- The completion/hang failure mode of an MMU invalidate (kayfabe leaving `TRIGGER`
  set → the guest spins to a 4 s/30 s RM timeout) presents as a **whole-guest hang**,
  not an in-guest FAIL line. That is caught only by the outer budget — which is why
  the harness rule below is load-bearing.

## 6. How to run it — and "a timeout IS a crash"

```
# build (module + initrd), from a box with kernel headers:
bash scripts/advguest/build_adv_guest.sh                 # cold reach
ADV_POST_INIT=1 bash scripts/advguest/build_adv_guest.sh # post-init reach

# one run under a hard budget (needs a kayfabe-linked qemu at $QEMU_BIN):
bash scripts/advguest/run_adv_guest.sh adv 30
ADV_POST=1 ADV_STORM=8192 ADV_THREADS=8 bash scripts/advguest/run_adv_guest.sh advp 45
```

`run_adv_guest.sh` inherits `run_fast_guest.sh`'s central rule: the whole run gets
**one budget** covering QEMU start, kernel, driver load, the suite and poweroff, and
a `timeout` (rc 124/137) is reported as **CRASH**, never "slow". A VMM that hangs
inside a trap handler wedges the vCPU on its MMIO exit and no further guest code runs;
this budget is the only thing that catches that. It also enforces the stale-binary
refusal (a `.rs` newer than the linked qemu ⇒ refuse), and derives the device line
and shared `memfd` RAM backing exactly as the fast lane, so the two cannot diverge on
the thing under test.

The verdict logic follows `the_last_line_is_not_the_verdict`: the harness does not
read the tail. It asserts `ADVGUEST_BEGIN` is present (suite started), `ADVGUEST_TOTAL`
is present (suite did not die mid-run — absence is detectable precisely because BEGIN
was printed first), and then gates on `fail=0` plus the module's own
`ADVGUEST_VERDICT=PASS`, which the counting code writes.

### Verified vs. built (honest status)

- **Built and compiled** `[measured]`: the module, `build_adv_guest.sh`,
  `run_adv_guest.sh`. `advguest.ko` builds clean against `6.8.0-59-generic` on `vh3`
  (`make RC=0`; `modinfo` shows the three params). Every register offset is cited to
  kayfabe source or the ogkm 580.159.04 headers and cross-checked against the shim.
- **NOT yet run** against a live emulated GPU. The phase/verdict logic, the liveness
  probe and the host-side grep are written and argued but unmeasured end-to-end. The
  `want=` names are the refusals each case is *designed* to provoke; which ones
  actually fire (and whether any case reveals a real defect) is the first thing to
  measure on a kayfabe-linked qemu box.

## 7. Layout

```
scripts/advguest/
  advguest.h          register map + ogkm struct helpers, one citation per number
  advguest.c          the suite (phases 0-3, ~25 cases), tally owned by the counter
  Makefile            out-of-tree kbuild
  build_adv_guest.sh  builds advguest.ko + a no-disk initrd (cold or post-init)
  run_adv_guest.sh    one run under a hard budget; a timeout IS a crash
```

It is deliberately parallel to `scripts/fastguest/` and shares no files with it: the
thin guest is the team's iteration loop and must keep working untouched.
