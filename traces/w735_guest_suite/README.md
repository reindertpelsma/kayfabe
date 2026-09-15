# w735 — the 30-arm raw-client suite IN THE GUEST: what it reports, and the wall it hits

**Measured 2026-09-15**, vast instance `51090077` (RTX 3060 / **GA106**, NVIDIA **open**
`580.159.04`), guest Ubuntu 24.04.5, tree revision **`d6201633`** (boot `w735a`) and
**`bfa143fe`** (boot `w735probe`).

⊘ **This is not a licence.** `SINGLE_STORE_PLAN.md` §7's deletions are gated on the GUEST suite
(`THE_CONSTRAINTS.md` §w724g), and the gate is **`SUITE_UNMEASURED == 0`**. It is 26.

## 1. The ledger — `run_w735a_suite.out`

    SUITE_ARMS=30 SUITE_PASS=4 SUITE_FAIL=0 SUITE_TIMEOUT=0 SUITE_UNMEASURED=26
    SUITE_RECOVERIES=26 SUITE_RECOVERED=0
    --concurrency PASS · --timer PASS · --engines PASS · --doorbell-census PASS

★ **`SUITE_FAIL=0`.** Every arm that reached its subject passed. Nothing in the suite found a
defect in what it tests; the suite could not reach 26 of its subjects at all.

## 2. Two of the three "known failures" were the HARNESS — `run_w735a_userns_probe.txt`

    GUEST_WHOAMI=ubuntu uid=1000
    GUEST_APPARMOR_USERNS=1
    GUEST_USERNS_AS_USER=DENIED
    GUEST_USERNS_AS_ROOT=ok

`--concurrency` and `--engines` are the **only two of the thirty arms that reach R10** (every
other arm returns from its own `if want_*` block first — checked at all 30 dispatch sites), and
R10 spawns a child with `ChildSpec::in_new_namespaces()`. The guest is Noble, which ships
`kernel.apparmor_restrict_unprivileged_userns=1`; the hook ran the ladder as `ubuntu` while the
host 30/30 reference and the graded `--uvm-mean` boot both ran under `sudo`.

⇒ **The "delta" was root-versus-`ubuntu`, not host-versus-guest.** With the uid corrected,
`R10 isolate = 4 workers`, `R11 through-isolate` and the **`R16 sandboxed doorbell`** all pass
*in the guest*.

⊘ `--gpu-info-sweep` is **not** the wedging arm and never was. It is the fifth arm, and it is
`UNMEASURED` rather than `TIMEOUT`: the device was already unopenable when it started.

## 3. ★★★★★ The real defect — `RmInitAdapter` is not repeatable, and the failure latches WPR2

`run_w735probe_open_ordinal.out` — the open-ordinal probe w424 asked for:

    OPEN 1..4  rc=0     the device opens
    OPEN 5     rc=124   ⇐ THE WALL. It does not refuse; it HANGS (30 s timeout)
    OPEN 6..8  rc=1     unopenable — `openat` now fails fast

★ `boot_capture.sh` spends one cycle on its own `nvidia-smi` before the hook, so the probe's
open #1 is really cycle #2 ⇒ **five `RmInitAdapter` cycles succeed per QEMU lifetime and the
sixth hangs.** Boot `w735a` agrees independently: `nvidia-smi` + exactly four passing arms.

`run_w735a_guest_nvrm_*.log` — the guest driver's own words at the wall:

    NVRM: _memdescSetSubAllocatorFlag … NV_ERR_INVALID_STATE @ mem_desc.c:404
    NVRM: … @ kern_bus_gm107.c:1798 / :1413
    NVRM: kbusInitBar2_HAL … NV_ERR_INVALID_STATE @ kern_bus_gm107.c:332
    NVRM: RmInitNvDevice: *** Cannot initialize the device
    NVRM: RmInitAdapter failed! (0x24:0x40:1220)

and then, for the rest of the QEMU's life, on **every** open:

    NVRM: _kgspBootGspRm: unexpected WPR2 already up, cannot proceed with booting GSP
    NVRM: RmInitAdapter failed! (0x62:0x40:2028)

⇒ the failed BAR2 init leaves **WPR2 up**, which our emulated GSP never clears.
⊘ **The guest cannot recover.** `modprobe -r nvidia_uvm nvidia_drm nvidia_modeset nvidia` +
`modprobe` + `nvidia-modprobe` was run **26 times** and reopened the device **zero** times —
`SUITE_RECOVERIES=26 SUITE_RECOVERED=0`, every reload landing on *"WPR2 already up"*, visible in
`run_w735a_guest_nvrm_after.log`. **The leaked state is ours, not the guest RM's.**

⊘⊘ **It is NOT w424's `ce_utils.c:304` CeUtils-scrubber chain**, which this campaign has carried
as the explanation of the device-open wall since. Same symptom, different cause — read the w424
note as superseded for this build.

## 4. Why this is a product defect and not a test problem

A guest that can open `/dev/nvidia0` five times and then never again is broken for anything
real: a CUDA process that restarts, a container runtime, a second tenant. The 30-arm suite is
not stressing the device — it is the **first workload that counted**.

⚠ And it is invisible to every grade this campaign has recorded, because each of them opens the
device once or twice. `W392D_GUEST_OUTCOME=(P)` is one client on one boot.

## 5. ⊘⊘⊘ The previous ledger would have called this boot GREEN

The old `rmladder_suite.sh` gated its exit on `if [ $n_fail -gt 0 ] || [ $n_to -gt 0 ]`, and a
cascaded arm incremented `n_skip` — which is in neither term. ⇒ `PASS=4 FAIL=0 TIMEOUT=0
SKIP_CASCADE=26` printed **`SUITE_RC=0`**, and that is **exactly the shape this boot produced**.
A caller checking the exit code would have recorded a green 30-arm guest suite in which 26 arms
never reached their subject — under a comment reading *"a caller cannot record a green by
ignoring the body."*

⚠ Nobody bought that pass; the arithmetic did. `SUITE_UNMEASURED` is now in the gate, which is
why renaming `SKIP/cascade` was not cosmetic.

## 6. ★★★ WITH THE CASCADE CONTAINED, THE SUITE REPORTS 30 VERDICTS — `w735b_rows.txt`

`[measured 2026-09-15, rev a21fbe41, `w735_suite_batched_run.sh w735b 3` — 10 boots, 3 arms each]`

    W735B_ARMS=30 W735B_PASS=26 W735B_FAIL=4 W735B_TIMEOUT=0 W735B_UNMEASURED=0
    W735B_ACCOUNTED=30   W735B_BOOTS=10

★ **26 of 30 arms pass inside the guest** — including `--ce-client` (*"ALL ARMS MET"*), the
CPU-writes-vidmem → CE-DMA → CPU-reads-back round trip that had never been run in a guest,
`--uvm-mean`, `--concurrent-fuzz`, `--cross-client-leak`, `--missing-page-fault` and
`--bar1-crossing`.

⊘ **`--gpu-info-sweep` PASSES.** The arm this campaign recorded as a device-wedging timeout does
nothing of the kind; it passes in ~seconds when it can open the device.

⊘⊘ **And the four non-passing rows above are NOT FAILures** — that ledger was wrong and is fixed
(w735m). `137` is `timeout -k`'s escalation-to-SIGKILL exit, i.e. a **TIMEOUT**; and two of the
four (`--atomics-probe`, `--pce-mask-probe`) followed a *killed* `--gpga-reserve-probe`, hung
**inside the device open**, printed nothing but their own `RMLADDER ARGV` line, and never reached
their subject at all. ⇒ the wall's first signature is a **hang**, not a refusal, and the
containment now classifies on the ladder's own `R2 version` marker rather than on an exit code.

## 7. ★★★ THE FOUR NOT-PASSING ARMS, EACH ALONE ON ITS OWN BOOT — `run_w735d_isolated_arms.out`

⊘ **A DIAGNOSTIC, NOT A RE-GRADE.** One arm per fresh QEMU, `RMLADDER_ARM_TIMEOUT=600` instead
of the graded 90 s, asking one question: *slow, or stuck?* The graded default is unchanged, and a
pass here is a statement about **speed**, not a suite result.

| arm | alone, 600 s | reading |
|---|---|---|
| `--atomics-probe` | **PASS** | ⇒ its batch row was **collateral** of the arm before it |
| `--pce-mask-probe` | **PASS** | ⇒ same |
| `--gpga-reserve-probe` | **TIMEOUT(137)**, still mid-sweep | a real defect — see below |
| `--ce-client-guest-ram` | **TIMEOUT(137)**, last line `DOORBELL-STORE #1 … ★★★ WROTE` | a real defect — see below |

⇒ **With the collateral removed the guest verdict is 28 PASS / 2 TIMEOUT / 0 FAIL**, and the two
that remain are *named*:

**(a) `--gpga-reserve-probe` — the bulk framebuffer read is at least 120× too slow.** It
memcpy-sweeps a **256 MiB** object and had not finished after **600 s** ⇒ **< 0.43 MiB/s**. The
same shape measured on the host through a device view of the reserved object is **52.5 MiB/s**
(`SINGLE_STORE_PLAN.md`, w734). ⚠ This is the arm that most directly exercises what §3 is
about, and it is the one the guest cannot complete.

**(b) `--ce-client-guest-ram` — a CE copy whose SOURCE is guest RAM rings its doorbell and the
completion never arrives.** Its last line is the isolate's own
`DOORBELL-STORE #1 host_token=0x00000003 ★★★ WROTE — the store instruction executed`. ⇒ the
submission happened and nothing came back, for **600 s**. ⊘ Its sibling `--ce-client` — the same
round trip with a **vidmem** source — **PASSES** (*"ALL ARMS MET"*), so this is specific to the
guest-RAM source path, and `HOST_DMESG_XID=0` on that boot: **no host fault explains it.**

## 8. What the containment does, and what it does not

`rmladder_suite.sh` now recovers-and-retries and reports **30 rows either way**, with
`UNMEASURED` (never reached its subject) kept distinct from `FAIL` (ran and judged itself
failed) — and `SUITE_RECOVERIES` vs `SUITE_RECOVERED` printed together, which is what turned
*"the arms cascaded"* into *"the wedge survives a guest driver reload"*.
`w735_suite_batched_run.sh` gets 30 real verdicts across several boots.

⊘ **Both are containment. Neither is a fix, and §7 stays unlicensed until the wall is gone.**
