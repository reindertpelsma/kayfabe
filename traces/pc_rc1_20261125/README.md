# pc hardware run of the RC1 candidate (kf3, no bind), 2026-11-25

STATUS: RESEARCH, 2026-11-25. MEASURED on one box, one die, one image. Preregistered in `PREREGISTRATION.md` (committed before the first
measurement). Labels: MEASURED = a file in this directory named in the row; INFERRED = reasoning, not seen in a log.

Run: the owner's pc (RTX 4070 12 GB, host driver 595.91.07, display manager up), pc run of 2026-11-25, 04:27Z to 05:14Z (the pc logs local time,
UTC+1). **No GPU bind, no reboot, no driver reload.** Source revision of every measurement below: **`237349e04c38d685e04fc8b84e8e39f25c236795`**,
head of `claude/rc1-b2-windows-display-20261125` at the start of the run (RC base plus PR #604, `unmapped-reads-zero` on/off/auto, and the B2 change,
the Windows refusal at fn 1). The branch head moved to `77a231617` during the run (the refusal is seated ahead of the identity observer, same
guest-visible reply, per the coordinator); that head was NOT tested. Built by `scripts/bench/build_kf3.sh` from a fresh depth-1 checkout into
`kf3-bins/237349e0/` (kept on the pc). Image: the sealed non-proprietary `win11_base` (Windows 11 Pro 25H2, NVIDIA 580.88), one throwaway child
overlay per boot of the 2026-11-24 `prep.qcow2`, 8 vCPU, `fb-mb=4096`, `guest-driver=580.65.06`, arm B CPU flags, network off, 300 s observation per
boot, argv and QMP event recorder as `traces/pc_displayexp_20261124/` (`displayexp.py` here is that script with the arms renamed; `chain.sh`
runs them; `summarize.py` makes `windows/summary.json`).

QEMU tree (MEASURED, pc run 2026-11-25, `gates/qemu_tree_state.txt`): the shared QEMU 10.2.4 tree on the pc is not a git repository; its
`hw/i386/pc.c` already contains the optional hyperv-wake hunk (`KF_HV_LO` present, `patch -R --dry-run` of the shipped patch does not apply
cleanly, so the hunk is there in a form that differs slightly). The binary is therefore stock kf3 plus that hunk. The hunk acts only on an ACPI S3 wake,
which no arm here performs.

## Results (MEASURED, pc run 2026-11-25, `237349e0`)

| step | result | file |
|---|---|---|
| build (`build_kf3.sh`) | **compiles and links**: `KF3_BUILT .../kf3-bins/237349e0/qemu-system-x86_64 rev=237349e0`; `kf3.c` (with `DEFINE_PROP_ON_OFF_AUTO`) built without error or warning | `gates/build_tail.txt` |
| `v3_gates.sh` | **9/9**, gate 10 PASS, 12 channel births all user, 0 refused: `V3_GATES_EXIT pass=9 fail=0 births_ok=1 gate10=PASS`. None of the four named refusal lines (`PRIVILEGED CHANNEL`, `CHANNEL BIRTH`, `CHANNEL CLASS`, `CUDA THREAD`) in the log | `gates/v3_gates.log` |
| fast suite, `KF_DEVICE=kf3 fast_suite.sh pcrc1 180` (fast guest rebuilt with `KF_FROM_HOST=1`, raw client built at the revision) | **30 PASS, 0 FAIL, 0 CRASH, 0 NOTRUN** | `fast/fast_suite.out`, `fast/fast_guest_build.log` |
| H-b, QEMU logs of the 30 arms | **0 of 30** logs contain `through our reservation`, `HELD BY HOST`, `HELD-BY-HOST` or `HELD-BY-OURSELVES`; 28 logs have exactly one `kf3: unmapped-reads-zero=auto`; the two fault arms (`defer-liveness`, `missing-page-fault`) have no such line and their argv carries `unmapped-reads-zero=off` | `fast/fast_unmapped_lines.txt` |
| H-b, host dmesg (cut by its monotonic stamp, gates tail plus the 30 arms) | **0** `mmuWalkMap` lines; 5 Xid 31 `qemu-system-x86` MMU faults, all within the time of the two fault arms (those arms test a real unmapped-read fault); 106 `pool_alloc.c:605` `NV_ERR_NO_MEMORY` assertion lines (see Unexpected) | `fast/host_dmesg_fast_window.txt` |
| W-default x3 (`display=on,gop=on,gop-efi`, no fill argument) | 3 of 3: log has `kf3: unmapped-reads-zero=auto` once; no `through our reservation`, no `HELD`; host `mmuWalkMap` 0; alive at 300 s; adapter `NVIDIA GeForce RTX 4070` Code 0, status OK; guest `nvidia-smi` names the RTX 4070 (580.88); `nvlddmkm` 153 = 0; no bugcheck | `windows/Wdef{1,2,3}/`, `windows/summary.json` |
| W-fill-on x2 (`unmapped-reads-zero=on`) | 2 of 2: host `mmuWalkMap: Failed to map VA Range 0x120000000 to 0x20f9fffff. Status = 0x51` once per boot, and the kf3 line `kf-host: map 0x120000000+0xefa00000 through our reservation 0xcafe0023: host RM answered 0x51 (no memory) ... REFUSED, not held by the host`; no `HELD BY HOST`. Guest: alive at 300 s, adapter Code 0, `nvidia-smi` names the RTX 4070, `nvlddmkm` 153 = 0, no bugcheck | `windows/Wfill{1,2}/` |
| W-display-off x2 (`-vga none -display none`, no `display=on`/`gop=on`) | 2 of 2: log line `kf3: WINDOWS GUEST REFUSED: the guest declared the Windows driver and this device has no display plane (display=off) ...` **exactly once per boot** (no repeat in 300 s); adapter `NVIDIA GeForce RTX 4070` **Code 43**, status Error (Device Manager problem 43); guest `nvidia-smi` fails ("couldn't communicate with the NVIDIA driver"); guest alive and answering ssh at every poll to 300 s; no bugcheck, no `SHUTDOWN` before our own quit, no reset, no `nvlddmkm` event at all (153 = 0, any = 0), no Kernel-Power 41 | `windows/Woff{1,2}/` |
| W-display-off, `windows-support=off` x1 | 1 of 1: **no** refusal line (as expected); the same picture: Code 43, `nvidia-smi` fails, alive at 300 s, no bugcheck, no `nvlddmkm` event | `windows/Woffsup1/` |

In the pc run of 2026-11-25 at `237349e0`, the end of every boot was our own `quit` (`SHUTDOWN` reason `host-qmp-quit`); nothing was measured beyond 300 s. Eight boots, order Wdef1 Woff1 Wfill1 Wdef2
Woff2 Wfill2 Wdef3 Woffsup1 (`windows/chain.out`).

## Verdicts
- H-build: **holds** (compiles).
- H-gates: **holds** (9/9).
- H-fast: **holds** (30/30).
- H-b: **holds** for the QEMU logs (0 of 30) and for `mmuWalkMap` (0). The premise of #604 "a passing run has no in-reservation 0x51" is not falsified on this pc.
- H-Wdef (#604 a): **holds, 3 of 3.** With the default there is no host `mmuWalkMap` failure, no refusal and no `HELD` line, and the driver works. The Windows profile
  turns the fill off under `auto` (the `kf3: unmapped-reads-zero=auto: ... FAULT for the spaces of a guest under the Windows support profile` line).
- H-Wfill (#604 b): **holds, 2 of 2.** The new refusal-by-name line appears in place of `HELD BY HOST`. The guest then does not suffer: it is alive, Code 0, `nvidia-smi` works
  (not the unpredicted bad outcome; INFERRED only that the 0x51 here is harmless to this image over 300 s).
- H-Woff: the refusal line is **present** (once per boot, 2 of 2) and the preregistered falsifier of "the refusal is better than the old behaviour" is **not met**: no bugcheck, no
  reboot loop, no hang in 2 of 2 boots over 300 s. The adapter ends in Code 43 with no working driver, which is the documented purpose of the refusal.
  Decision rule: **B2 stays in the RC on these data.** Limits: 300 s, one die, one image, one binary.
- H-Woffsup: recorded only (no refusal line, Code 43, alive, no bugcheck).

## Unexpected and open (MEASURED unless marked INFERRED)
⊘ **Correction 2026-11-25 (release integrator), to item 1 below.** The inference in item 1 (the old bugcheck came from the fill, not from the missing display plane) is
   contradicted by evidence already in the tree, and this run cannot test it. `traces/pc_hwgates_20261124/README.md` (pc run 2026-11-24, kf3 `63e46069` and `ed41beea`, all eight boots display-off)
   has the guest ending early and `nvlddmkm` 153 with the fill OFF (rows r2 and r7, `unmapped-reads-zero=off`) and on the pre-fill binary (rows r4 and r6, `ed41beea`), with the adapter at
   Code 0. In the three display-off boots of this run (pc run 2026-11-25, `237349e0`) the driver never started (Code 43: refused at fn 1 in two boots, no Windows profile in the third), so a
   driver that starts and then resets was not observable here. What this run adds is the guest-side effect of the refusal: Code 43, no `nvlddmkm` event, alive at 300 s.

1. **The 2026-11-24 display-off bugcheck did not reproduce.** `traces/pc_displayexp_20261124/` (pc, 2026-11-24, `63e46069`) measured arm A (display off, default fill ON) as
   `nvlddmkm` 153 and bugcheck 0x113 in 3 of 3 boots at 50-170 s. Here (pc 2026-11-25, `237349e0`) both display-off boots, and the `windows-support=off` boot, stay alive for 300 s with 0
   `nvlddmkm` events. The `windows-support=off` boot carries no refusal line, so the refusal line is not what changed the outcome. INFERRED, not tested: the cause of the old bugcheck was the fill
   (default ON then, `auto` = off for the Windows profile now) and not the missing display plane; a display-off boot with `unmapped-reads-zero=on` on this binary would test that (not run).
   If that is right, the text of the B2 refusal ("NVIDIA's Windows driver resets itself without one") is not supported by this run; the refusal itself is harmless here.
2. **Every Windows boot (display on or off, fill default or on) and every Linux fast arm logs host `pool_alloc.c:605` `NV_ERR_NO_MEMORY` assertions**, 3 per Windows boot at about 4 s
   after the QEMU start in the default boots (`windows/*/host_dmesg_nvrm.txt`; 18 in the two fill-on boots) and 106 in the gates-plus-30-arms window. They are not `mmuWalkMap` failures (the
   assertion text names `VASPACE_FLAGS_RETRY_PTE_ALLOC_IN_SYS`). Not investigated, no control run without kf3 here.
3. The guest `nvidia-smi` in W-display-off fails and the adapter is in problem 43, i.e. the refusal does not "not start" the adapter: the driver loads and fails its start (INFERRED reading of
   Code 43); the brief expected "not started".

## Not run
`windows-support=off` only once; no display-off boot with the fill forced on; no S3; nothing past 300 s; no other die or driver; the head `77a231617` was not built or booted; Linux fast suite
not repeated with the other default values; host `nvidia-smi` once at the end (after everything was stopped).

## Idle and restore (MEASURED, pc 2026-11-25)
Scratch (`pc_rc1_20261125` under the bench, the winvm overlay dir, the cargo target on tmpfs, the fast-suite logs of this tag) removed; no `qemu`, `swtpm` or broker process; `pc_gpu.lock`
released; `gdm` active; host driver 595.91.07 loaded, `nvidia-smi` names the RTX 4070; `/workspace` 28 GB free before and after. START/END lines written (pc and coordinator `PC_ACTIVE.log`,
04:27Z and 05:14Z). `kf3-bins/237349e0/` stays. The shared `qemu-build-kf3` build dir was rebuilt by this run (standing convention).

## Method notes
- The first H-b host-dmesg check of this run used a line offset into the ring buffer, which had wrapped; it was discarded and redone by monotonic timestamp (the numbers above).
- The Windows poll counts `nvlddmkm` 153 only. `windows/*/qemu_key_lines.txt` is the filtered key QEMU lines; `host_dmesg_nvrm.txt` is the NVRM lines of the boot (ioctl noise removed).
