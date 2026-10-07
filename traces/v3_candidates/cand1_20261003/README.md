# Candidate v3-cand-1 on real hardware — merge bar, apps, display (2026-10-03/04)

**SUPERSEDED FOR RESUMPTION, 2026-10-04:** the B5 fix and the full candidate were
subsequently tested at `0ac157b2`: B5 4×14/14, full merge bar and the app baseline
passed. See `../cand1_20261004/README.md`, recovered after this handoff. This
directory remains the evidence of the earlier failure at `8a682f1b`.


⊘ **2026-10-04 (later): the B5 failure below is root-caused and fixed at `c1ca7945` (see *Candidate or
master?*); this directory still records `8a682f1b` only, and the fix needs its own hardware run.**

**STATUS: ANSWERED, 2026-10-04 01:30 UTC.** Stage 1 (merge bar) PASS, stage 2 (apps) PASS,
stage 3 (display) B0/B1/x11-dispsw PASS and **B5 FAIL** — the B5 failure reproduces with master's
code on the same box (see *Verdict*).

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

Guest display userspace: `provision_guest_gfx.sh` then `provision_guest_display.sh all` into the
box's `guest.qcow2` (after the merge bar had built its fast guest from it; `GUEST_GFX_DONE rc=0`,
`GUEST_DISPLAY_DONE rc=0`, X driver and `nvidia-drm-outputclass.conf` present —
`apps/provisioning/guest_{gfx,display}.log`). Driver script `display_job.log`: the recipe of
`traces/v3_display/gop_final_20261003/gopfinal_runs.sh` **without its build step** (lane.sh runs
`kf3-bins/8a682f1b`, the merge bar's binary; every boot's `run_*_rev.txt` reads
`kf3-bin-rev:8a682f1b`), then the x11-dispsw pair through `dispsw_run.sh` exactly as
`traces/v3_display/dispsw_20261003/README.md` runs it. Host kernel lines for the whole stage:
`host_kernel_journal.log` (journald) — **0 Xid, 0 NVRM**. Every display boot's guest `dmesg` is
non-empty and carries 25 `NVRM` lines (`*/run_*_dmesg.log`).

### GOP lane — B0, B1 pass; B5 FAILS (2–3 of 14 arms), and master fails it identically on this box

| lane | command | result |
|---|---|---|
| **B0** (`b0c1/`) | OVMF, `gop=off` | **PASS**: probe pixel-exact 1920x1080, 120/120 flips at 61.11 Hz, `B0_SEED_LINES=0`; the console shows NOTHING until head 3 (+63649 ms), and the lost scanout goes black after the 250 ms hold at shutdown (the documented `gop=off` behaviour) |
| **B1** (`b1c1/`) | OVMF, `gop=on`, timed shots | **PASS**: `DISPLAY_BOOT_HANDOFF black_frames=0 first_window=[+65033 ms … head 3]`, lane rc 0; probe pixel-exact, 120/120 flips at 60.00 Hz; B2's lines: *"the guest preserves a firmware console of 0x7f0000 bytes … 3 regions"*, the seed *"retired at the first change: the console is ONE run"*, re-seeds at seed life 2 and 3, `cannot preserve` 0 times, `boot[frames=1107 retired=+65032ms]`. `shots/contact_sheet.png`: placeholder, TianoCore (25–30 s), kernel text (40–50 s), login prompt (52–75 s, across the handoff), the probe's pattern (90 s) — no black shot at the handoff |
| **B5** (`b5c1/`) | OVMF, `gop=on`, `unload_hook`, `KF3_DISPLAY_TRACE=1` | **FAIL**, lane rc 3: `DISPLAY_B5_VERDICT FAIL arms=14 failed=3` — `B5A2_AFTER_SESSION`, `B5A2_CONSOLE`, `B5A2_PRESERVED`; the other 11 arms (B5C, B5C2, B5C3, the Wayland arm (a), B5B_*, B5_TEARDOWNS `physical_writes=5 restored=5`) pass |

**What fails.** Arm (a2): X11 Cinnamon on the NVIDIA X driver (`modeset=0`), then lightdm stopped.
NVKMS restores the console (`+170628 ms the console shows head 3 … window 6 iso 0x10088`), and the
device then logs **`kf3: display: scanout REFUSED context DMA 0x10088 on channel 7: NotBound`**
before the guest frees the window channels with PRESERVE_HW; at the free head 3 has no window, so
kf3 shows *"BLACK … (the scanout shown is lost: no head is lit)"* instead of *"the PRESERVED
scanout"*, and the text console never comes back (`b5c1/hook/b5_device.log`,
`b5c1/run_b5c1_qemu.log.gz`).

**Candidate or master? — master, on this box** (`b5_repeats/`, driver `b5ab_job.log`, alternating
runs, same box, same guest image, same recipe):

| run | kf3 binary | B5 verdict | `scanout REFUSED context DMA 0x10088` | PRESERVED line |
|---|---|---|---|---|
| `b5c1` (stage 3) | `8a682f1b` (candidate) | FAIL, 3 arms | 1 | 0 |
| `b5r1` | `8a682f1b` | FAIL, 2 arms (`B5A2_CONSOLE`, `B5A2_PRESERVED`) | 1 | 0 |
| `b5m1` | **`d4c3767b` = master `789dee9f`'s code** | FAIL, the same 2 arms | 1 | 0 |
| `b5r2` | `8a682f1b` | FAIL, the same 2 arms | 1 | 0 |
| `b5m2` | **`d4c3767b`** | FAIL, the same 2 arms | 1 | 0 |

`kf3-bins/d4c3767b` is the binary this box's own merge bar built for `v3-gop-unload` (`mb_gop`,
2026-10-03 19:59–20:20), and `git diff d4c3767b 789dee9f -- crates qemu firmware` is empty, so it
runs master's code; the master runs used scripts from a worktree at `789dee9f`. ⇒ **Not a
regression of this candidate**: master fails B5 the same way, with the same refusal, on vmb. The
display code on that path is identical in both (`git diff 789dee9f 8a682f1b --
crates/kf-qemu/src/display.rs crates/kf-disp/src/inst.rs crates/kf-disp/src/scanout.rs` is empty).
`B5A2_AFTER_SESSION` passed in the four repeats only because the session shot was the fallback
dialog there (`b5a2_x_1280.png`) and any black frame then counts as "changed"; the after-session
screen is black in all five runs.

⊘ **ROOT CAUSE CONFIRMED and FIXED, 2026-10-04 — `c1ca7945` on `v3-cand-1`, not yet re-run on a box.**
The inference below holds, read from these logs and the source rather than assumed: b5c1's and
run_h/b5h's QEMU logs carry the same display events from the restore to the core free (window 6
latches `0x10088`; *"chn 7 PUT 0xfa4: 0 effects"*, NVKMS's flip to NULL with no UPDATE; the window and
core frees with `preserve: true`), b5c1 adding five GSP `Free` RPCs and the refusal. The order is
NVKMS's `nvFreeDevEvo` (`ogkm-580: src/nvidia-modeset/src/nvkms-evo.c:9101-9112`): restore the console,
free the console surface (RM clears the context DMA from display instance memory with the window still
armed on it), then free the channels with `PRESERVE_HW`. The refused copy cleared
`ScanState::last_plan` (*"copied whole"*), so the preserving free kept nothing. The fix keeps each
window's context DMA as its ARMED state latched it until the window latches again (`LatchedDmas`,
`crates/kf-qemu/src/display.rs`; `docs/design/V3_DISPLAY.md` §4.11.13's ⊘⊘ block), with a GPU-free test
of the ordering that fails with per-copy resolution. B5 must be re-run at the candidate's new head.

Why B5 passed for `06b307c4` on vast 54032077 (`traces/v3_display/gop_final_20261003/`) and fails
here — an inference from the code and both logs, not a run: each scanout copy re-resolves every
window's context DMA (`crates/kf-qemu/src/display.rs:2106`, `io.resolve(so.client, so.handle,
so.chn)`), and with nobody watching the console a copy is due every 250 ms (`display.rs:1997`). The
guest unbinds the console's context DMA during the teardown, before it frees the windows. On
54032077 the restore-to-preserve window was 153 ms (+149459 → +149612 ms, no copy in between); here
it is 346 ms (`b5r1`: +169046 → +169392 ms) to 2.2 s (`b5c1`), so a copy lands in it, refuses the
window, and the PRESERVE_HW free finds no window. Hardware keeps scanning a latched surface after
its context DMA is unbound, so kf3's re-resolution looks like the defect; it is master's, and it is
timing-dependent.

### X11 desktop — the x11-dispsw A/B pair: as documented

`DISPLAY_X11_BARE=1 dispsw_run.sh dsw_a_off_c1` (A, default = `x11-dispsw` off) and
`… dsw_b_on_c1 x11-dispsw=on` (B); SeaBIOS, `display=on`, host probe `kfdsw_probe.ko` loaded around
each (`dsw_*/host_kfdsw_trace.log`). Both lanes rc 0.

| | A: off (`dsw_a_off_c1/`) | B: on (`dsw_b_on_c1/`) | the dispsw README's criteria (runs 9/10) |
|---|---|---|---|
| X driver | `(EE) NVIDIA(0): Failed to allocate display software resources.` | absent (`last_EE` = `Failed to get virtual display support info`, the unchanged one) | off: present / on: absent |
| Cinnamon X11 | segfault in `libnvidia-glcore`, fallback dialog (`desk_1_1280.png`) | **up, 0 crashes**, panel + wallpaper | off: segfault / on: up |
| X11 vkcube IMMEDIATE / MAILBOX / FIFO; long run | 134 / 1 / 134; 134 | **0 / 1 / 0; 0** (`desk_vkcube_1280.png`: the cube in a Cinnamon window) | off 134/1/134 / on 0/1/0 (MAILBOX unsupported by the NVIDIA X11 WSI either way) |
| glxgears vsync / no-vsync | 42.2–42.7 / 41.1 FPS | **57.9–59.6 / 2591 FPS** | on: ~60 / 2000–2700 |
| bare Xorg: glxgears windowed / fullscreen, vkcube FIFO | 44.6–46.3 / **2.0** / RC 134 | **59.8–60.0 / 58.8–59.8 / RC 0** | off: fullscreen ~2 FPS, RC 134 / on: ~60, RC 0 |
| guest Xid, `waiting for GPU progress`, crashes | 0, 0, 1 (cinnamon) | 0, 0, 0 | 0 |
| host Xid (journald), host NVRM lines | 0, 0 | 0, 0 | 0 |
| `dispsw[...]` at the end | — (off) | `twins=80 live=0 host_refused=0 no_twin=0 capped=0 id_refused=0 repaid=0 withdrawn=0 other_sw=0 free_refused=0` | `live=0`, nothing refused |
| software classID read back = the guest's | — | 80 of 80 twins | every twin |
| GSP refusals that differ A vs B | `0x20800a5d` refused | answered | the one difference |
| host CPU-RM display-SW release path (probe) | 0 calls; 1 628 531 GSP event drains | 0 calls; 1 696 779 drains | 0 calls |
| `0x90720101` (`NOTIFY_ON_VBLANK`) | — | 0 | 0 |

Both lanes also log one `Flip event timeout on head 0` per X server start
(`DISPLAY_FLIP_EVENT_TIMEOUTS_AFTER=2`), unchanged from the documented runs.

## A host-kernel warning new with this candidate (not a failing check)

`host_pat_warnings.txt` (journald, per run window on this box): the host kernel logs
`x86/PAT: kf3-vamgr:<pid> freeing invalid memtype [mem 0x10…]` — 42 lines at 22:59:02 at the end of
the merge bar's `gpga-reserve-probe` arm, 2 in the app runs, 52 in the display stage, 1 in one B5
repeat. The same windows hold **47** for `v3-scratch-bound`'s own merge bar (`69ccb08c`, 19:13–19:35)
and **0** for master's code (`d4c3767b`: its merge bar and both `b5m` runs) and for `v3-sec-nonpriv`
(`1d71f3db`). ⇒ it arrives with the `v3-scratch-bound` lane (a PAT-tracked mapping unmapped in a
different shape than it was reserved, from the VA-manager thread). No check reads it and no run
failed with it; not investigated further here.

## Verdict

At `8a682f1b415844801e855ca6b28388f91c244251`: **merge bar PASS, apps PASS (no regression vs the
61/65 baseline), display: B0, B1 and the x11-dispsw A/B PASS, B5 FAIL.** The B5 failure reproduces
with master's own code on the same box (2 of 2 runs), so it is a pre-existing master defect exposed
by this box, not a change this candidate made; by the task's rule (all three stages must pass) the
run's verdict is still **fail**.
