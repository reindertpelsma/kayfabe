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

## 5. What the containment does, and what it does not

`rmladder_suite.sh` now recovers-and-retries and reports **30 rows either way**, with
`UNMEASURED` (never reached its subject) kept distinct from `FAIL` (ran and judged itself
failed) — and `SUITE_RECOVERIES` vs `SUITE_RECOVERED` printed together, which is what turned
*"the arms cascaded"* into *"the wedge survives a guest driver reload"*.
`w735_suite_batched_run.sh` gets 30 real verdicts across several boots.

⊘ **Both are containment. Neither is a fix, and §7 stays unlicensed until the wall is gone.**
