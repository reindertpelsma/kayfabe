# Candidate v3-cand-1 on real hardware — merge bar, apps, display (2026-10-03/04)

**STATUS: IN PROGRESS, 2026-10-04 00:55 UTC.** Stage 1 (merge bar) and stage 2 (apps) are done
and passed. Stage 3 (display) is running on the box; this file is updated after it.

**Tested commit (every run below):** `8a682f1b415844801e855ca6b28388f91c244251` (branch `v3-cand-1`,
"merge v3-dispsw-exp"). This directory is evidence only: the commits that add it change no code.

**Box:** vast 54049598 (`vmb`), RTX 3060 (GA106, 12 GB), 19 vCPUs, 49 GB RAM, KVM template, host
driver 580.159.04 (open kernel module), host kernel 6.8.0-59-generic. The box pulled the commit from
GitHub (`/root/kayfabe`, `git checkout --detach 8a682f1b…`, clean). One GPU job at a time.

**CI at the same commit:** GitHub Actions runs 37157600752 (push) and 37157724581
(workflow_dispatch), both `success` at head `8a682f1b`, all four jobs (stable, aarch64, slow,
firmware).

## Stage 1 — the merge bar (`merge_bar/`)

Command, from the checkout, with a target directory of its own (fresh, created by this run, so no
artifact of another checkout could be reused):

```
CARGO_BUILD_JOBS=$(nproc) CARGO_TARGET_DIR=/workspace/bench/kf-target-cand1 \
  bash /root/kayfabe/scripts/bench/box/merge_check.sh v3-cand-1 cand1mb
```

`merge_check.log` is the script's own log: `START 2026-10-03T22:38:04`, `HEAD=8a682f1b4158…`,
`EXIT rc=0 2026-10-03T23:01:02`.

| item | result | log |
|---|---|---|
| revision (the script's own worktree) | `8a682f1b415844801e855ca6b28388f91c244251` | `merge_check.log` |
| every kf-* crate test | **1936 passed, 0 failed** | `tests.log` |
| v3 gates | **9/9**; `V3_GATES_BIRTHS births=11 user=11 refused=0 ok=1` | `gates.log` |
| kf3 built from this revision | `KF3_RC=0`, `kf3-bins/8a682f1b`, GOP driver embedded | `kf3.log` |
| bare-metal raw-client suite | **30/30**, 0 crash | `host.log` |
| fast guest rebuilt | `FG_RC=0` | `fg.log` |
| 30-arm thin-guest suite, kf3 | **30/30, 0 crash, 0 not run** | `suite.out` |
| census self-test (planted logs) | 11 cases, 0 wrong | `census_selftest.log` |
| channel-birth census | `BIRTH_CENSUS_OK arms=30 suite_births=161 gate_births=11 all PRIVILEGED_CHANNEL=0 privilege=USER` | `births.log` |

Guest kernel logs: every one of the 30 thin-guest boots left a non-empty kernel console log
containing `NVRM` lines (22–234 per arm) and no `Xid` line — `guest_console_nvrm.txt` (one row per
arm, counted on the box from `fast_cand1mb_<arm>_ttyS0.log`), the 30 logs themselves in
`guest_consoles_ttyS0.tgz`.

## Stage 2 — apps (`apps/`)

The matrix of `docs/design/V3_APP_MATRIX.md` §5, in the shape of the 61/65 baseline
(`traces/v3_cdp/app_matrix_2830988f/`, `V3_CDP.md` §5.4: one boot, no PM, then every non-PASS app
alone). Provisioning (`provisioning/`, all from the checkout at `8a682f1b`): `build_bundle.sh` on
the box (CUDA 12.6, llama.cpp `4b1a27fa0`, bundle sha `fb64de820389b448`, no `BUILD_*=FAIL`),
`setup_side.sh` on the host (all `SETUP_*=ok`), the guest image = a copy of the box's
`guest.qcow2` (`guest_apps.qcow2`, Ubuntu noble 6.8.0-142, stock 580.159.04) provisioned with
`provision_guest_apps.sh` (all `SETUP_*=ok`). Then, serially:

```
bash scripts/apps/apps_matrix.sh host cand1 all
KF3_BIN=/workspace/bench/kf3-bins/8a682f1b/qemu-system-x86_64 KF_GUEST_IMG=/workspace/bench/guest_apps.qcow2 \
  APPS_PER_BOOT=100 bash scripts/apps/apps_matrix.sh guest cand1 all
```

kf3 = the merge bar's own binary (`kf3-bins/8a682f1b`, sha256 `01da8ddccfbd6737…`); every boot
logged `BINARY kf3-bin-rev:8a682f1b`; device `fb-mb=8192`, 16 GiB guest RAM, 6 vCPUs.

| run | result (2026-10-03/04, 23:52–00:46 UTC) | baseline `2830988f` |
|---|---|---|
| host (bare metal) | **71/71**, 0 host Xid in that window (journald) | 71/71 |
| guest, ONE boot, no PM | **67/71 = 61/65 apps + 6/6 stream probes** | 61/65 + 6/6 |
| guest, each non-PASS app alone | the same 4 fail alone | the same 4 |

- **App by app, every row equals the baseline** (`apps_vs_2830988f.txt`): 71 rows, 0 verdict
  differences in host, batched or alone; **0 apps that passed at `2830988f` fail here** ⇒ no
  regression.
- The four failures are the documented UVM demand-paging four (`UnifiedMemoryStreams`,
  `UnifiedMemoryPerf`, `conjugateGradientUM` — the silent `Error amount = 1.000000` —
  `attach_verify`), with the documented signature: kf3 `RC host twin … except_type=0x1f (Xid 31)`
  (`triage.txt`, `triage_isolated.txt`), host `Xid 31` MMU faults on GRAPHICS (8 lines: 4 in the
  batched boot, 1 in each alone boot — `host_xid_journal.log`, read from journald for the window),
  guest Xid 0; each passes on bare metal.
- Output digests equal the host's and the baseline's: `torch_correct` cnn_train_step
  `763c693a5a53f948`, `hf_generate` `0d973108a6251e14`, `llama_cpp_gen` `caf613a5d956b7e3`
  (`guest.dig`, `host.res`).
- Guest kernel logs: every boot's `dmesg` (taken after `nvidia` loaded) is non-empty and carries
  `NVRM` (25 lines; the end-of-boot `dmesg_after` 94–1478) — in `all_logs.tgz` (every per-app log,
  the boot logs, the kf3 logs as `.zst`).
- ⚠ **Harness finding, not kayfabe:** `boot_capture.sh`'s host-dmesg delta read **0 lines, 0 Xid**
  for the batched boot (`boot_ap_cand1_b1.probe.log`: `watermark 1769 → 1717`, `HOST_DMESG_XID=0`)
  while journald holds 4 host Xid 31 lines inside that boot. The box's kernel ring buffer is full
  and wrapping, so its line count fell during the boot and the `tail -n +watermark` delta was empty.
  Every host-Xid number in this directory is therefore read from **journald by time window**, not
  from `run_*_hostdmesg.log`.

## Stage 3 — display (`display/`)

Queued after stage 2: the GOP lane (B0, B1, B5) and the x11-dispsw A/B pair.
