# V3: one Windows 11 desktop boot on a rented Vast box (the verification plan for the owner's bar (b))

STATUS: DESIGN-ONLY, 2026-10-31. Research and design for the Windows 11 lane that Codex will operate. Nothing here was run: no rental, no box, no VM, no secret. The only live check is a header-only HTTP probe of the vendor download URLs from a GPU-free sandbox (section 4). Every number is labelled `MEASURED` (with run, date and path), `CODE READ`, or `INFERRED`. An unlabelled sentence in a table cell inherits the label of its row's source column.

Owner ruling, 2026-10-31 (given to this task, not yet in `docs/OWNER_RULINGS.md`): no `next` to `master` merge until Windows is verified. The bare-metal Windows host is gone, so Windows is verified on rented Vast boxes. Minimum bar (b): ONE full Windows 11 LTSC (Microsoft evaluation) desktop boot on a Vast KVM box through `scripts/bench/windows/win_vm.sh`: install once, NVIDIA driver 580.88 with the GSP keys, `OverlayTestMode=5`, desktop reached, `nvidia-smi` plus a small D3D or compute probe pass, and no TDR in a bounded idle-plus-probe window.

## 0. Answer first

- **Does it fit the 3 h lease?** On the success path, yes, with slack: the stage anchors below sum to about 90 min (best), 110 min (typical) and 150 min (slow box) of a 175 min timer. INFERRED from MEASURED stage times taken on different boxes (section 5). It does **not** fit a second full install behind a late failure, and there is no two-lease route that is cheap and legal today (section 5.4).
- **Confidence.** MEDIUM for the time model (each stage has a recorded anchor; no single lease has recorded the whole chain). LOW for "bar (b) passes on the first lease": the lane's best Windows record on a rented box is a 600 s idle hold on an RTX 4090 that was followed by a failed app matrix (section 6.4), and two other dies storm under load (section 3.3).
- **The scripts do not yet do the whole job.** On `origin/next`, `win_vm.sh` installs Windows and installs the driver with GSP forced, but it does not (1) set `OverlayTestMode=5`, (2) sign in a desktop session, (3) export the 19 `KF3_*` flags the recorded Code 0 holds used, (4) run any acceptance probe, (5) bound its waits to a 3 h lease. Section 3 lists these with proposed diffs. Section 2 is written so that an operator can run the lane with the scripts as they are plus a few guest commands, and says where a script change is better.
- **The five host assumptions that matter most** (section 3.1): the unbounded GPU-lock wait, the unbounded install wait, no desktop sign-in or `OverlayTestMode`, no `KF3_*` flags from `run`, and a hard-coded ssh connect timeout that the lane scripts already try to override.

## 1. What exists, and where

CODE READ, 2026-10-31, `origin/next` at `e1dbdbd41`, plus the named unmerged branches.

| Piece | Where | State |
|---|---|---|
| Unattended install, check boot, driver stage, `run` boot loop | `scripts/bench/windows/win_vm.sh` (846 lines) with `autounattend.xml.in`, `firstlogon.ps1`, `check.ps1`, `gsp_on.ps1`, `collect.ps1` | on `next` |
| User guide | `scripts/bench/windows/README.md`, `docs/howto/windows-guest.md`, `docs/WINDOWS_GUEST_SETUP.md` | on `next` |
| Fresh box to sealed Windows image, one script with a stage log and `WINLANE_TIMING` | `scripts/bench/box/windows_lane.sh` plus `WINVM_INSTALL_CACHE=unsafe` in `win_vm.sh` | branch `claude/tur1-winlane`, draft PR #165 per `traces/tur1_20261027/README.md:83`; not on `next` |
| Headless kf3 device (`WINVM_KF3_DISPLAY=off`) | `win_vm.sh` | branch `claude/tur1-headless-winvm`, draft #163 (`traces/tur1_20261027/README.md:81`); not wanted for this bar, which needs a console screenshot |
| Bind-then-seal-then-arm kit: `bind.sh`, `arm.sh`, `enable580_once.ps1`, `accept580.ps1`, `diag_min.ps1`, `p19.flags` | `traces/blackwell_ab_20261023/ROUND_B1_1830/run_kit/` and `traces/sweep_windows_20261021/box_scripts/` | branch `claude/bw4-b1n-20261024`; reads `/root/sweep2/common.sh` and a Blackwell readiness guard, so it is a pattern to copy, not a script to run |
| Cheap host reference text | `traces/tur1_20261027/iter2_ga106/hostref_before.txt` | evidence on `next` |
| Windows CUDA ladder probes (`cup2`, `cup3`, `cup8`) and D3D probe (`kf_dxprobe`) | `scripts/bench/windows/cuda/build_cup_win.sh`, `scripts/bench/windows/appmatrix/build_tools.sh`, `.../tools/src/kf_dxprobe.cpp` | on `next`; cross-built with mingw-w64 on the box |
| Provider-fault classifier | `scripts/ci/vast_payload.py:35-43` (`provider_gpu_unavailable`) | on `next` |

Not usable on a rented box (owner-host only): `windows_broker.sh` (needs `/var/lib/kf-windows-20261005`, `/opt/nvkvm-broker`, `loginctl` seat0), `winpass_cycle.sh` (checks IOMMU group `0000:01:00.0`), `kfagent_ogl/launch-2990*.sh` (`/var/lib/vast-windows-test-media`), `diesweep/win_die.sh` (OVMF and baseline image from the owner host), `signin.py` as written (see 3.2, item 5).

## 2. Step list: rent to teardown

`T` is minutes since the rent call. "Gate" is a go/no-go the operator records in its log. All commands run ON the box as root unless marked `[local]`. The box pulls code from GitHub; only text comes back (`scripts/bench/box/README.md:21-22`). Commands are the repository's own; where a step needs a command that does not exist yet it says `NEW` and section 3 proposes it.

| # | Step | Command / script | Artifact | Gate |
|---|---|---|---|---|
| 0 | Budget and timer `[local]` | Read the live credit; require credit >= 8 USD plus 3 h x price; arm `scripts/bench/autodestroy_box.sh <id> 10500 <log>` (175 min) as soon as the id exists | `credit_before.txt`, `autodestroy-<id>.log` | credit floor; one own box only |
| 1 | Rent `[local]` | `vastai search offers 'vms_enabled=true num_gpus=1 gpu_name=<die> disk_space>=120 cpu_cores_effective>=16 inet_down>200' -o dph` then `vastai create instance <offer> --image docker.io/vastai/kvm:ubuntu_cli_22.04-2025-05-16 --disk 120 --raw`, output filtered to `success` and `new_contract` (`scripts/bench/box/README.md:36-39`) | `own_id.txt` (the id, nothing else) | `df -h /` >= 60 GB; `/dev/kvm` present; `nvidia-smi -L` shows one GPU. Else destroy by id and re-rent (README:43-46) |
| 2 | Preflight | `bash scripts/bench/host_preflight.sh` (exit 2 = host fault) | `preflight.txt` | exit 0. Exit 2 is `PROVIDER`; destroy |
| 3 | Host reference, before | the lines of `traces/tur1_20261027/iter2_ga106/hostref_before.txt`: `df -h /`, `ls -l /dev/kvm`, `nproc`, `free -g`, `lspci -nn`, `nvidia-smi` name/driver/memory, BAR1 total, Xid count, a 1 MiB driver-API round trip | `hostref_before.txt` | green: `CUPROBE_OK roundtrip_equal True`, Xid count 0 |
| 4 | Provision (Windows lane) | `KAYFABE_BRANCH=<branch> HOST_DRIVER=595.91.07 bash windows_lane.sh` (from #165) = `provision_box.sh`, fetch `NVIDIA-Linux-x86_64-580.159.04.run` (the bench tree wants it at a fixed path), `provision_host_driver.sh`, `provision_bench_tree.sh`, `build_kf3.sh`, `apt install ovmf swtpm swtpm-tools sbsigntool python3-virt-firmware xorriso netpbm p7zip-full`, `win_vm.sh fetch` in parallel | `/root/prov/winlane.log`, `WINLANE_TIMING`, `kf3-bins/<rev>/` | `WINLANE_EXIT rc=0`; host driver is the open 595.91.07 |
| 5 | Probe build (parallel with 6) | `apt install gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64`; `bash scripts/bench/windows/cuda/build_cup_win.sh <out>`; `bash scripts/bench/windows/appmatrix/build_tools.sh <out>` (Ubuntu 22.04's mingw lacks `D3D_FEATURE_LEVEL_12_2`: add `-D'D3D_FEATURE_LEVEL_12_2=((D3D_FEATURE_LEVEL)0xc200)'`, per `traces/tur1_20261027/README.md:59`) | `probes/cup3.exe cup8.exe kf_dxprobe.exe` plus their sha256 | exes exist; hashes logged |
| 6 | Install Windows | `WINVM_SMP=<=16> WINVM_INSTALL_CACHE=unsafe WINVM_QEMU=/workspace/bench/kf3-bins/<rev>/qemu-system-x86_64 bash scripts/bench/windows/win_vm.sh install` | `logs/install.log` (`START`/`EXIT` per step), `installcheck_check.txt`, sealed `disk/base.qcow2` | `WINVM_INSTALL_DONE` and `WIN_CHECK_DONE fail=0` by T+75 or stop (3.2 item 2) |
| 7 | Sign the boot-display ROM | `bash win_vm.sh sign` (builds `kf-gop-export` with cargo, `sbsign`s it with this host's db key) | `KF_GOP_SIGNED ... sha256=` line | line present |
| 8 | Bind boot | `unset` every `KF3_*` variable, `export` the flag list (3.1 item 4), then `bash win_vm.sh run --overlay bind --arm B --guest-driver 580.65.06 --once --max-boots 1 --tag bind --detach`; wait for ssh; `bash win_vm.sh nv-install --no-enable` | `bind_run.log`, `nv_install.log`, `nv_install_*.txt` (`GSP_ON readback ... EnableGpuFirmware=1`) | both readbacks are 1; `DriverVersion 32.0.15.8088` |
| 9 | Desktop preparation (NEW) | over ssh in the same boot: `OverlayTestMode=5`; autologon for the harness account with the passwordless-device fix (3.2 item 1); read every value back | `desktop_prep.txt` | all read-backs match |
| 10 | Seal the bind parent | `bash win_vm.sh stop`; `chmod 0400 disk/bind.qcow2`; `sha256sum` | `bind_parent_sha256.txt` | clean `guest-shutdown`, no QEMU left |
| 11 | Host reference, middle (cheap) | `nvidia-smi`, Xid count | `hostref_mid.txt` | Xid delta 0 |
| 12 | Arm: cold boot, device disabled | child overlay of the sealed parent; same `run` line with `--overlay arm1 --tag arm1`; wait for ssh (ceiling 450 s, as `arm.sh` uses) | `arm1_b1_qemu.log`, `arm1_b1_serial.log`, `arm1_b1_qmp_events.jsonl` | ssh up; no `GSP REFUSED` line yet (device still disabled) |
| 13 | Enable once, record the instant | `pnputil /enable-device <id>` once (never replayed), then `Win32_VideoController` and `Get-PnpDevice` | `enable_once.txt`, UTC of the enable | `ConfigManagerErrorCode 0` within 120 s. Code 43 or 22 ends the arm (section 7) |
| 14 | Idle hold | 600 s, polling QEMU liveness and `GUEST_PANICKED` in the QMP events | `hold.log` (`ARM_HOLD_OBSERVED seconds=N`) | N >= 600 and QEMU alive |
| 15 | Probes and screenshot | `nvidia-smi.exe -L`, `-q`; the probes of step 5; `bash win_vm.sh shot arm1_desktop`; the TDR counter (section 6.2) | `probes.txt`, `nvidia_smi_*.txt`, `shots/*.png`, `tdr_counts.txt` | section 6.1 |
| 16 | Collect | `bash win_vm.sh collect arm1` (text only; dumps are listed, not copied); `dmesg`, Xid count, `nvidia-smi -q` | `logs/evidence/arm1/*`, `hostref_after.txt` | files present |
| 17 | Stop | `bash win_vm.sh stop` (guest shutdown, then QMP powerdown, then quit) | `stop.log` | no `qemu-system-x86` process |
| 18 | Export to git | compress the logs; transfer text only (`vget`, or `scp` to the operator); commit under `traces/windows_on_vast_<date>/lease_<id>/` on the PR branch | commit sha | secrets absent (section 6.3) |
| 19 | Teardown `[local]` | `vastai destroy instance <id> -y` (or `curl -X DELETE https://console.vast.ai/api/v0/instances/<id>/`), then list instances and require the id absent (README:194-200) | `teardown.txt` | API absence of the exact id; keep the auto-destroy armed until then |

**Checkpoints (INFERRED from the time model in section 5):** media and provisioning done by T+30; sealed install by T+75; sealed bind parent by T+100; no new arm starts after T+150 (the `latest-start guard: leave 25 min` that `arm.sh` itself enforces); hard stop T+170, with the 175 min auto-destroy as the net and the vast-reaper default of 180 min behind it (`ci/vast-broker/DEPLOY.md:372`).

A failed arm is repeatable: step 12 onward starts from a fresh child overlay of the sealed parent and costs about 25 min (section 5). A failed install is not.

## 3. Old-host assumptions and the change each needs

### 3.1 The five that matter most

1. **Unbounded GPU-lock wait.** `win_vm.sh:705-707` takes `/tmp/kayfabe-fastguest.lock` with `flock -w 7200`. A stale holder (a leftover `vx -b` job, a Linux suite) silently burns two of the three lease hours. Proposed:
   ```diff
   -    flock -w 7200 8 || die "the GPU lock $GPU_LOCK was not free in 2 h"
   +    flock -w "${WINVM_GPU_LOCK_WAIT:-120}" 8 || die "the GPU lock $GPU_LOCK was not free in ${WINVM_GPU_LOCK_WAIT:-120}s (holder: $(fuser -v "$GPU_LOCK" 2>&1 | tail -1))"
   ```
2. **Unbounded install wait.** `win_vm.sh:548` waits `vm_wait 14400` (4 h) for the install boot, longer than the lease. Past installs took 19 to 41 min (section 5). Proposed `vm_wait "${WINVM_INSTALL_TIMEOUT:-4200}"` (70 min); on timeout `vm_wait` already screendumps and QMP-quits (`win_vm.sh:437-442`).
3. **No desktop session and no `OverlayTestMode`.** `firstlogon.ps1:93-96` removes `DefaultPassword` and sets `AutoAdminLogon=0` on purpose, and `gsp_on.ps1` writes only the two GSP keys. A `run` boot therefore stops at the lock screen, and `DWM` runs without `explorer.exe`. `OverlayTestMode=5` is written only by the app-matrix runner (`scripts/bench/windows/appmatrix/winapps.py:324`) and the owner-host TDR hunt runners (`scripts/bench/windows/host_tools/tdrhunt/tdr-run20.sh:188`). Needed: a `desktop_prep` step (3.2 item 1).
4. **`run` exports no `KF3_*` flags.** `win_vm.sh:704` sets only `KF3_RPC_TRACE`. Without the production flags the driver starts, is refused `GFX_POOL_QUERY_SIZE` (0x2080121f) and ends in Code 43 (MEASURED 2026-10-27, lease 57882297, `traces/tur1_20261027/run3_windows/w4_refused_set_no_flags.txt`, `traces/tur1_20261027/README.md:27`). The 19 flags of the recorded Code 0 holds are in `traces/sweep_windows_20261021/p19.flags` on branch `claude/bw4-b1n-20261024` (the owner-host list `windows_broker_prod2.sh:285` has the same 19 plus `KF3_TIMER_MAP`, which no crate on `next` reads). All 19 names are still read by `crates/` on `next` (CODE READ 2026-10-31, `grep` over `crates/` and `qemu/`). Proposed: `run --env-file FILE`, which exports only lines matching `^KF3_[A-Z0-9_]+=[01]$`, refuses anything else, and prints the list in the `WINVM_RUN_START` line so the flag set is part of the evidence. UNSURE: whether this list is still the production list on `next` (`docs/STATUS_AND_HANDOFF.md:132` says a flag-free default route is prepared but its Windows result is pending).
5. **Hard-coded ssh connect timeout.** `win_vm.sh:473` uses `ConnectTimeout=8`. `windows_lane.sh` (#165) and the BW `bind.sh` (under `traces/`) both export `WINVM_SSH_CONNECT_TIMEOUT` (100 s in the former), but no file under `scripts/` on `next` or on `claude/bw4-b1n-20261024` reads it (CODE READ 2026-10-31). On a nested box a busy guest answers the ssh banner slowly, so `wait_ssh` (`win_vm.sh:479-492`) can report "never answered" for a guest that is up. Proposed `-o ConnectTimeout="${WINVM_SSH_CONNECT_TIMEOUT:-8}"`.

### 3.2 The full table

| # | Where | Assumption (old host) | Change for a nested Vast box |
|---|---|---|---|
| 1 | `firstlogon.ps1:93-96`, `autounattend.xml.in:101` (`LogonCount 1`) | A person signs in, or a broker window is attached. Autologon is switched off at the end of first logon. | New `desktop_prep.ps1`, piped to `ssh ... powershell -NoProfile -Command -` on stdin so the password never appears in argv: read `secrets/win_password` on the host; set `Winlogon` `AutoAdminLogon=1`, `DefaultUserName=kf`, `DefaultPassword`, `DefaultDomainName=.`; `PasswordLess\Device\DevicePasswordLessBuildVersion=0`; `Dwm\OverlayTestMode=5`; read each back. The `0` value is the recorded fix for Windows resetting autologon on the next boot (MEASURED 2026-10-08, owner-host run 58, `scripts/bench/windows/windows_broker.sh:188-191`; the cause is inferred). The password is the box's throwaway; it lives in the guest registry on an untrusted box, the same exposure as the native-baseline preparation of `scripts/bench/windows/appmatrix/native/kf_native_prep.ps1:2-13`. Fallback if autologon fails: `signin.py` over QMP and QGA. |
| 2 | `scripts/bench/windows/signin.py:35` | The account is `vast`: `endswith("\\vast")`. | The harness account is `kf` (`win_vm.sh:75`). Take the user from `KF_GUEST_USER` (default `kf`). Its docstring says the caller holds the GPU lock and sets `KF_GUEST_PW`. |
| 3 | `win_vm.sh:60`, `:65`, `:63-64` (`/workspace/winvm`, `/workspace/bench`, `cache/`) | A large local disk at fixed paths. | Keep the paths: `provision_box.sh` creates `/workspace/bench`. Add a free-space refusal before `fetch` (about 10 GB of media and extracted driver, an 80 GB sparse base that grows with the install, overlays). INFERRED: 60 GB is enough, 120 GB requested. |
| 4 | `win_vm.sh:166`, `:159-164` | The kf3 QEMU lives at `kf3-bins/<rev>` where `<rev>` has a `-dirty` suffix if any tracked file is modified. | Do not edit scripts on the box. Pass `WINVM_QEMU=` explicitly (as `windows_lane.sh` does). |
| 5 | `win_vm.sh:76`, `:362`, and no `-uuid` or `-smbios` anywhere in `qemu_common` (`:348-372`) | One fixed MAC (`52:54:00:4b:46:57`) and the QEMU default SMBIOS identity on every VM. `docs/howto/windows-guest.md:109` and `troubleshooting.md:75` say a unique SMBIOS UUID, serial and MAC per VM, offline until OOBE; Windows Autopilot once matched a default-identity VM to an unrelated company tenant. | Offline is already enforced here: `restrict=on` gives the guest no route out (`win_vm.sh:361`), so Autopilot cannot be reached. INFERRED. Still add per-VM identity: generate a UUID, serial and MAC suffix once into `manifest.env` at install and reuse them in every boot (`-uuid`, `-smbios type=1,serial=`, `mac=`). This also makes kf3's `gpu-uuid=auto` deterministic: `qemu/hw/misc/kf3/kf3.c:175-178` hashes QEMU's `-uuid`, and with none it uses a random value (CODE READ), so today every QEMU restart of the same VM gets a different GPU UUID. UNSURE whether that matters to the driver; owner question 9. |
| 6 | `win_vm.sh:352-353`, `:68-69` (8 GiB RAM, 8 vCPU) | A host with RAM to spare (the owner-host rule was to check before a second VM). | Nothing in the script needs 29 GiB. One VM runs at a time; the box needs about 8 GiB for the guest plus the build. Require `free -g` >= 24 total. The vCPU count does not speed the install (it is bound by one nested-virtualization thread, INFERRED in `traces/tur1_20261027/README.md:91`); keep 8 for `run`. |
| 7 | `win_vm.sh:71`, `:699` (`fb-mb=8192`, `bar1-size=134217728`) | A 12 GiB card. | `fb-mb` is real host VRAM: `WINVM_KF3_FB_MB=6144` on an 8 GB card (`traces/tur1_20261027/README.md:58`); the Linux fast guest uses the card's memory less 2 GiB (`scripts/fastguest/run_fast_guest.sh:227-230`). The 128 MiB BAR1 fits the common 256 MiB host aperture (`docs/howto/requirements.md:57`). A realize refusal on BAR1 is the documented Vast exception (section 7). |
| 8 | `win_vm.sh:705-707` | See 3.1 item 1. | |
| 9 | `win_vm.sh:210-222` (`apparmor_swtpm`), root | Ubuntu jammy with an enforced swtpm AppArmor profile, run as root. | The Vast KVM image is jammy and the lane runs as root; the same step ran on `vwin` and on later leases (`traces/v3_windows/discovery_20261004/install/install.log`). No change. |
| 10 | `win_vm.sh:361` (`user,restrict=on,hostfwd=tcp:127.0.0.1:2224-:22`), `:368` (`-display none -vnc unix:`) | No tap, bridge or display needed; ssh on a fixed loopback port. | Right for a box: no tap, no bridge, no X. Needs `--enable-slirp`, which `build_kf3.sh` sets. Port 2224 conflicts only with a second VM. `provision_bench_tree.sh:96-103` also creates `nvktap0` and NAT rules for the Linux lanes; harmless, unused by Windows. |
| 11 | `provision_host_driver.sh:164` (`display-manager sddm gdm3 lightdm` loop) | A desktop host whose display manager holds the GPU. | The Vast KVM image has none. No change. |
| 12 | `win_vm.sh:773-776`, `README.md:95` | The 2.6 GB `Display.Driver` is copied into the guest over slirp. | MEASURED rates differ 14x: 18 min (2.4 MB/s, `vwin`, 2026-10-04, `traces/v3_windows/discovery_20261004/nv1/nv_install.log`) against 76 s (lease 57882297, 2026-10-27, `traces/tur1_20261027/run3_windows/nv_install.log`). The `scp` timeout is 1800 s (`win_vm.sh:477`). Optional: build an ISO of `Display.Driver` with `xorriso` and attach it as a CD in the bind boot (needs a `WINVM_QEMU_EXTRA` hook). Do this only if the measured copy rate is under 5 MB/s. |
| 13 | `win_vm.sh:800-812` (`cmd_stop` never kills) | A kf3 exit census is worth keeping, so QEMU is never killed. | On a hung guest (a Code 43 guest ignored `shutdown /s` for 5 minutes, `V3_WINDOWS_DISCOVERY.md:276`) the operator waits at most 300 s, then QMP `quit` and records `UNCLEAN`. The box is destroyed afterwards, so the census is the only loss. |
| 14 | `win_vm.sh:13-18`, `README.md:39` ("about an hour on a nested box") | One hour. | Recorded installs are 19 to 41 min; the README figure is high. Not edited here. |
| 15 | `scripts/bench/box/provision_full.sh` | A full Linux bench (fast guest, raw client, suites). | Not for this lane. Use the `windows_lane.sh` stage list, which skips the fast-guest build. |
| 16 | `win_vm.sh:704` (`KF3_RPC_TRACE=1` default) | A measurement flag on by default. | Keep it for the bar: the GSP-path signal in section 6.1 needs the kf3 log, and `--no-rpc-trace` exists if the log is too large. Compress before export. |

### 3.3 The GPU die decides the outcome more than any script

Not a host assumption, but the largest single risk to bar (b). From `docs/design/V3_FAMILY_COVERAGE.md` (a transcription of the 2026-10-27 family audit; its labels are kept: `MEASURED Mb` = on a branch, not on master).

| Die (card) | Windows result on a rented box | Label and source |
|---|---|---|
| AD102 (RTX 4090) | Code 0, DWM and Explorer, 600 s idle hold clean; app matrix 13 PASS, 4 FAIL, 1 NOTRUN of 112 | MEASURED 2026-10-22, lease 57193877, `traces/sweep_dies_20261018/lease_57193877/ADA_RESULT.md` |
| GB205 | Code 0 and a 601 s idle hold with an unmerged fix; display dumps 0x193/0x1A8/0x1B8 still appear | MEASURED Mb, revision `007ba1eb0` |
| GB202 (RTX 5090) | Code 0 then bugcheck 0x113 (BW6, 2026-10-24); a 600 s clean hold on a different machine (BW11, 2026-10-26) | MEASURED Mb, `traces/bw11_20261026/README.md` |
| AD106 | historical Code 0; workloads PARTIAL | MEASURED / CLAIMED, revision `b0de7572` |
| AD104 (RTX 4070 class) | stable only on the owner host, which is gone; no rented-box Windows run | CLAIMED, no revision |
| TU106 (RTX 2060 SUPER) | Code 0, then 66 TDR events in 8 min; two fresh boots, two different outcomes | MEASURED 2026-10-27, lease 57882297, `traces/tur1_20261027/README.md:35` |
| GA106 (RTX 3060) | Code 0, then 54 events at 6 min and 116 RM unloads in 17 min | MEASURED 2026-10-27, lease 57908255, `traces/tur1_20261027/README.md:99` |
| GA102 | driver stage BLOCKED, bind readiness inconclusive | MEASURED Mb |
| GB206 | Code 43, later Code 0 then Xid | CLAIMED |

Consequence for the plan (INFERRED): rent AD102 first (best record, highest price), or AD106 or GB205 if an offer is cheap; do not spend the one attempt on TU106 or GA106, whose cheap price is the only point in their favour. If only a storming die is on offer, the lane should not start (owner question 3).

## 4. Image acquisition

CODE READ, `win_vm.sh:86-96` (URL, sha256 and byte count are pinned together; `cmd_fetch` at `:175-190` refuses a size or hash mismatch), and a header-only probe on 2026-10-31.

| Medium | Source | Bytes | sha256 (pinned) | Header probe, 2026-10-31 |
|---|---|---|---|---|
| Windows 11 Enterprise LTSC 2024 evaluation, en-US x64, build 26100.1742 | Microsoft fwlink `linkid=2289029` (redirects to `software-static.download.prss.microsoft.com/.../26100.1742.240906-0331.ge_release_svc_refresh_CLIENT_LTSC_EVAL_x64FRE_en-us.iso`) | 5112850432 | `67cec5865eaa037a72ddc633a717a10a2bed50778862267223ddb9c60ef5da68` | 302 then 200, `Content-Length: 5112850432`, `Last-Modified: 2024-09-19` |
| virtio-win 0.1.302 | `fedorapeople.org/groups/virt/virtio-win/direct-downloads/archive-virtio/virtio-win-0.1.302-1/` | 877373440 | `303f7ae40dad495d6ae474fdc571df58958a4dbc5c37a522d80f9a203867949d` | 200, `Content-Length: 877373440`, `Last-Modified: 2026-08-27` |
| Win32-OpenSSH 10.0.0.0p2 MSI | the project's GitHub release | 6586368 | `ddec9c53864280759cf9f74791cefd387100e3946aa849a1c138a4ed1b96b7d9` | not probed |
| NVIDIA 580.88 desktop DCH installer | `us.download.nvidia.com/Windows/580.88/580.88-desktop-win10-win11-64bit-international-dch-whql.exe` | 890807536 | `90c49f925b41ee062e02cc8094a16b6d057abfde4e85235a76d63ffe267f63a9` | 200, `Content-Length: 890807536`, `Last-Modified: 2025-07-28` |

The three probed sizes equal the pinned byte counts. A matching size is not a matching hash: only the hash check on the box decides. The `Last-Modified` of the virtio file is later than the pin date of the first run, so it was re-uploaded at the same size; the first fetch hash is the authority.

- **Verification.** `win_vm.sh fetch` hashes every medium and dies on mismatch. A mismatch is classified `MEDIA`, never kayfabe's (section 7); the lane cannot repair it in-lease, because moving a pin is an owner action ("change the URL and the hash together", `win_vm.sh:81-85`).
- **The `inputs.lock.json` style of the `win-gpu-image` repository** was not read: that repository is not in this checkout, and no credential was used to fetch it. UNSURE what its lock records beyond hash and size. The in-repo equivalent is the `MEDIA` array plus the manifest each VM writes (`make_layout`, `win_vm.sh:195-208`: `MEDIUM=<file>:<sha>` lines).
- **No binary is copied into git.** Each box downloads straight from the vendor. Nothing is mirrored by us.
- **Download size and time.** MEASURED 2026-10-21, lease 57064037, `traces/sweep_windows_20261021/lease_57064037/preparation/final_setup_receipt.json` (branch `claude/bw4-b1n-20261024`): the whole media step took 158 s (14:41:10 to 14:43:48 UTC) for 6.9 GB, ISO 120 s, virtio 14 s, NVIDIA 23 s, hashing included. Other recorded media steps were shorter because the files were already cached. The driver is then extracted (`7z x ... 'Display.Driver/*'`, 2.6 GB, 45 s in MEASURED 2026-10-27, lease 57882297).
- **Licence.** The Microsoft image is a 90-day evaluation intended for IT professionals; use it for testing only and rebuild rather than keep (`docs/DOWNLOADS.md:83`). `docs/DOWNLOADS.md:68` records its terms as unclear (ISO licence unread; "cache, owner only"). The NVIDIA driver terms are listed there as not yet reviewed (`docs/DOWNLOADS.md:83-86`). kayfabe ships no vendor binary (`docs/NOTICES.md:15`); this plan only fetches on the renter's box. The evaluation clock starts at install, so a saved image would also be running down.

## 5. Time and cost model

Anchors are MEASURED on other boxes and runs (2026-10-04 to 2026-10-27, the lease or run named in the last column); the sums are INFERRED. Times are minutes.

| Stage | Best | Typical | Slow | Anchor |
|---|---|---|---|---|
| Rent to a box that answers (create-to-running lag, ssh) | 5 | 5 | 8 | INFERRED. No recorded duration; the blob helper bounds the created-to-running lag at 299 s (`scripts/ci/kf-box-blob-url`) |
| Preflight and host reference | 3 | 3 | 3 | INFERRED |
| Provision through prerequisites | 14 | 16 | 25 | MEASURED 951 s for box 219, run-file 9, host driver 264, bench tree 405, kf3 18, prerequisites 36 (2026-10-27, lease 57908255, `traces/tur1_20261027/iter2_ga106/winlane.log`); 14 min on `vwin` (2026-10-04, `V3_WINDOWS_DISCOVERY.md:50`); 25 min by `provision_full.sh` (lease 57882297, `windows_lane.sh` header). Media fetch overlaps this |
| Install boot, check boot, seal | 22 | 35 | 44 | Install boot MEASURED 19 min (BW11, `traces/tur1_20261027/README.md:91`), 21.7 min (2026-10-21, lease 57064037), 30 min (2026-10-04, `vwin`), 32 min (lease 57882297), 41 min (lease 57908255, 16 cores, `cache=unsafe`); check boot 2.5 to 2.8 min (lease 57064037; `vwin`) |
| Sign, bind boot, driver copy, GSP keys, desktop prep, seal | 12 | 15 | 30 | boot to ssh about 150 s (`vwin`, `V3_WINDOWS_DISCOVERY.md:172`); extract 45 s; copy 76 s or 18 min; `gsp_on` 3 min 43 s (2026-10-27, lease 57882297); `sign` and prep INFERRED 3 min |
| Cold boot, enable, Code 0 | 6 | 8 | 10 | INFERRED; the arm's ssh ceiling is 450 s |
| Idle hold | 10 | 10 | 10 | MEASURED hold length in the recorded Code 0 runs: 600 s (2026-10-22, lease 57193877), 601 s and 600 s (2026-10-26, lease 57799086) |
| Probes, collect, shots | 8 | 8 | 10 | INFERRED; `accept580` is bounded at 240 s by its caller |
| Stop, export, teardown | 10 | 10 | 12 | INFERRED; `cmd_stop` waits up to 180 s, then 120 s |
| **Total** | **~90** | **~110** | **~150** | against a 175 min timer |

### 5.1 What a retry costs

- A failed **arm** (steps 12 to 17) restarts from a child overlay of the sealed parent: about 25 min. It fits in the best and typical cases. In the slow case it is marginal; the T+150 no-new-arm rule decides.
- A failed **install** detected by T+55 on a fast box costs 25 to 44 min plus the remainder (about 50 min): it fits only in the best case (about 125 min). Later than that it does not fit. Do not start a second install after T+75.

### 5.2 Cost

INFERRED from recorded rates. Rent is hourly: 0.107 USD/h (RTX 3060, lease 57908255, 2026-10-27), about 0.12 USD/h (RTX 2060 SUPER, lease 57882297), 0.447 USD/h observed for lease 57064037 (2026-10-21, `cost_midpoint.json`). Two older overnight leases billed 2.19 and 2.80 USD for 1.96 and 2.90 h (2026-10-21, `old_lease_invoices.json`), about 1 USD/h; the GPU of those is not recorded in the file. A 3 h lease therefore costs about 0.3 to 3.2 USD. The only cap on record is 12 USD per day "raised for 2026-10-25" with the 8 USD credit reserve kept (`docs/OWNER_RULINGS.md:1425`, section AN); whether it holds for 2026-10-31 is owner question 1. At the top of the range the cap allows three 3 h leases; plan for one attempt plus one retry lease.

### 5.3 If it does not fit: a duration exception

If the owner will not accept the retry-less plan, the cheapest change is a stated exception: 4 h, with the same checkpoints and the no-new-arm line moved to T+210. It adds at most 0.1 to 1 USD at the recorded rates and buys one more full arm.

### 5.4 Two-lease split with a saved image

INFERRED, not measured: the sealed `base.qcow2` is an 80 GB sparse image; a Windows 11 install is likely 12 to 20 GB used, and a compressed copy needs several parts under 2 GiB each (the per-asset limit of GitHub releases, an external fact to confirm). The only store on record is the private `win-gpu-image-private-store` repository that the Worker's blob route reads (`docs/design/VAST_SETUP_CACHE_20261023.md:45`, `scripts/ci/kf-box-blob-url`); per the TUR1 report its write path is the `win-gpu-image` environment only and its licence review is open (`traces/tur1_20261027/README.md:89`). So a second lease could in principle download parts at box speed and skip the 22 to 44 min install. It is not recommended now, for four reasons:

1. `win_vm.sh` never restores a VM from another VM's state (`README.md:87-88`); a restore needs a new, owner-approved subcommand.
2. The image is bound to per-VM secrets: the swtpm state, the harness ssh key, the Windows password, and the Secure Boot variables whose `db` carries this host's GOP certificate (`win_vm.sh:236-267`). A new box generates a new db key, so the stored variables and the new key would disagree. Whether the option-ROM policy tolerates that is unmeasured; the discovery run only showed that stock OVMF runs any option ROM (`V3_WINDOWS_DISCOVERY.md:92`).
3. Legal: the evaluation terms are unread and the stated policy is "rebuild images, do not keep" (`docs/DOWNLOADS.md:68`, `:83`).
4. The saving is at most one install; the second lease repeats the 14 to 25 min provisioning and adds a download whose speed is unmeasured.

## 6. Evidence and acceptance

### 6.1 Acceptance criteria (bar (b))

All must hold in ONE boot of ONE image; a retry from the same sealed parent is a new attempt and both attempts are reported.

| # | Criterion | Evidence file | Source of the check |
|---|---|---|---|
| A1 | The host reference is green before and after: stock driver visible, Xid count 0 before, 1 MiB round trip equal, Xid delta 0 after | `hostref_before.txt`, `hostref_after.txt` | `traces/tur1_20261027/run4_regress/host_reference_after.txt` shape |
| A2 | Install is sound: `WIN_CHECK_DONE fail=0` (edition Enterprise LTSC build 26100, Secure Boot true, TPM 2.0, BitLocker off, no VBS, no problem devices) | `installcheck_check.txt` | `check.ps1` |
| A3 | The GSP keys and `OverlayTestMode` read back as 1, 1 and 5, and the driver is 32.0.15.8088 (580.88) | `nv_install_*.txt`, `desktop_prep.txt` | `gsp_on.ps1:57-59` |
| A4 | The NVIDIA adapter reports `ConfigManagerErrorCode 0` | `enable_once.txt`, `diag_after.txt` | `enable580_once.ps1`, `diag_min.ps1` pattern |
| A5 | The kf3 log shows the GSP path: phase `Cold -> ProtectedRegionUp -> Booted -> Running`, `fn 72 GSP_SET_SYSTEM_INFO` with `bGspNocatEnabled=1`, and the `kf-rm` line that keys the Windows 580.88 guest as 580.65.06. Absent: `GSP REFUSED fn76/0x2080121f` (the no-flags Code 43 signature), `fn 47 UNLOADING_GUEST_DRIVER` and `Running -> Suspending -> Halted` inside the window | `arm1_b1_qemu.log` (grep results saved as `gsp_signals.txt`) | line shapes MEASURED 2026-10-04, `V3_WINDOWS_DISCOVERY.md:216-264`, at older revisions: adjust the patterns to the current log, do not drop the check |
| A6 | A desktop session exists: `dwm.exe` and `explorer.exe` running, the signed-in user is `KFWIN\kf`, and a PNG of the kf3 console is not the lock screen | `desktop_state.txt`, `shots/arm1_desktop.png` | `accept580.ps1:11-14` plus a screenshot |
| A7 | `nvidia-smi.exe -L` and `-q` run: the GPU name, driver 580.88, a firmware version that is not `N/A` | `nvidia_smi_L.txt`, `nvidia_smi_q.txt` | `collect.ps1` |
| A8 | At least one compute probe (`cup3` or `cup8`, graded `bad=0 maxerr=0` for `cup8`) and one D3D probe (`kf_dxprobe d3d11`, `KFDX RESULT OK`) pass. Run all of `cup2 cup3 cup8 d3d11 d3d12` and report each; the owner's minimum is one of each kind | `probes.txt` | `scripts/bench/cup8.c`, `kf_dxprobe.cpp` |
| A9 | No TDR from the enable instant to the end of the window (600 s idle, the probes, then 120 s settle): zero of Display 4101, `nvlddmkm` 153, System 41, 1001, 6008, WER LiveKernelEvent 141/193/113/117, and no `GUEST_PANICKED` in the QMP events; host Xid delta 0 | `tdr_counts.txt`, `arm1_b1_qmp_events.jsonl`, `hostref_after.txt` | zero-count definition as in lease 57193877's hold (`ADA_RESULT.md`: NVIDIA 153, Display 4101, bugcheck 1001 all 0) |
| A10 | Clean stop and teardown: guest `shutdown`, QEMU gone, the own id absent from the API list | `stop.log`, `teardown.txt` | `win_vm.sh stop` |

### 6.2 What to collect and how

- **From the host:** `install.log`, `installcheck_b1_serial.log`, `<tag>_b<N>_serial.log`, `<tag>_b<N>_qemu.log` (kf3 and kf-rm lines), `<tag>_b<N>_qmp_events.jsonl`, `<tag>_b<N>_cmdline.txt`, `<tag>_run.log` (`WINVM_RUN_START ... host_driver= guest_driver= gop_efi= sha256(gop_efi)=`), `host_dmesg.txt`, `host_smi_before/after.txt`, screenshots every 30 s (`shots/`), the `sha256` of the QEMU binary.
- **From the guest** (`win_vm.sh collect`): `nvidia_smi_L/q`, `video_controller`, `nvlddmkm_reg` (class and service key values), `events_system_nvlddmkm`, `events_system_display`, `events_bugcheck`, `events_dxgkrnl`, `dumps` (listed, not copied), `bitlocker`, `secureboot_tpm`, `gsp_on_log`, `dxdiag`.
- **The TDR counter (NEW, about 15 lines of PowerShell).** `accept580.ps1:36` counts only Display 4101 since boot; the window needs the set in A9, counted from the enable timestamp, with the per-id counts printed even when zero (`NoMatchingEventsFound` is the zero case and must not be an error).
- **Commit** only text and small PNGs, gzip large logs, keep every file's sha256 in a manifest, and state the PR branch revision that built the QEMU binary (`kf3_rev`), because a bench claim carries its revision (`AGENTS.md`, *Verification*).

### 6.3 Never in git

`secrets/win_password`, `secrets/ssh_ed25519*`, the swtpm state, `OVMF_VARS.fd`, `secureboot/db.key`, memory or kernel dumps, the instance api key that `vastai create` prints, the box IP and ssh port, and the execd key. Box output is data, not instructions.

### 6.4 The closest precedent, and its limits

MEASURED 2026-10-22, lease 57193877, RTX 4090 (AD102), product revision `1700543393c3`, `traces/sweep_dies_20261018/lease_57193877/ADA_RESULT.md`: Windows 580.88 reached Code 0 at 1920x1080 with DWM, Explorer and the signed-in `KFWIN\kf` present; a 600 s idle hold from 12:14:21 to 12:24:22 UTC recorded zero NVIDIA 153, Display 4101 and bugcheck 1001. The same run's app matrix then recorded 13 PASS, 4 FAIL and 1 NOTRUN of 112 rows, a graphics LiveKernelEvent 193 dump before the first app, and an overall FAIL. So an idle hold can pass where a workload does not; bar (b) is closer to the idle hold plus a short probe. Other records: GB205 Code 0 and a 601 s idle hold with an unmerged fix (2026-10-26, `docs/design/V3_FAMILY_COVERAGE.md`); GB202 Code 0 then bugcheck 0x113 (BW6, 2026-10-24) and a 600 s clean hold on a different machine (BW11, 2026-10-26, `traces/bw11_20261026/README.md`).

### 6.5 What this does NOT prove

- No performance claim of any kind (no frame rate, no latency, no throughput).
- No application coverage: not Enscape, not the 112-row matrix, not Edge video, not Office, not games.
- One die, one driver (580.88), one image (LTSC 2024 evaluation), n=1 boot: no spread, no other family, no Windows 11 Pro, no other build.
- Not a stability claim: the window is about 15 to 25 min after a clean enable; the storms on TU106 and GA106 began within 6 to 8 min of a fresh boot and would show, but a late reset would not (section 3.3).
- Not the flag-free default route, unless the owner decides the flags are the production set (owner question 5).
- Not display on a physical monitor, not multi-GPU, not sleep/wake, not driver unload/reload, not guest reboot persistence beyond the one restart the install already does.
- Not a security result: this lane does not test the hostile-guest rules.
- Not a Windows-native reference. A same-box native Windows run needs a separate lease (BW7 was a different lease on the same machine, `docs/design/V3_FAMILY_COVERAGE.md:234-238`).

## 7. Failure modes and how to classify them

Classes: `PROVIDER` (the rental is bad; never kayfabe's), `OPERATOR` (our script, image or configuration), `MEDIA` (a vendor file changed), `KAYFABE-KNOWN` (a defect already on record), `KAYFABE-NEW` (needs the reference below), `UNATTRIBUTED` (cannot tell).

**Attribution rule (CODE READ).** A failure on a rented box counts against kayfabe only if the same box's host reference is green before and after, or a native Windows reference on the same hardware passes (`docs/design/V3_FAMILY_COVERAGE.md:25-27`; `traces/tur1_20261027/README.md:5`). Without that, record `UNATTRIBUTED`. The Vast host BAR1 aperture limit is a documented exception: a BAR1 refusal counts only against a native reference that passes (owner rule recorded in the operator's notes, 2026-10-24; not found in `docs/OWNER_RULINGS.md`, so treat as owner-stated). A PASS needs no reference but is still reported with one.

| Symptom | Where it shows | Class | What the operator does |
|---|---|---|---|
| Offer stuck `loading` 30 to 45 min, `GPU error, unable to start instance`, `machine does not support VMs` | rent log; seen on the native-Windows path with 4090 and 5090 boxes (`docs/design/V3_CONSTANT_MATRIX_20261014.md:405`) | `PROVIDER` | Destroy by id, re-rent once. Do not count against the attempt budget |
| Root disk under 60 GB despite `--disk 120` | `df -h /` | `PROVIDER` | Destroy, re-rent (README:43-46) |
| No `/dev/kvm` | `ls -l /dev/kvm` | `PROVIDER` | Destroy |
| `host_preflight.sh` exit 2; `NOT_READY driver=No devices were found`; `DRIVER_RC=8`; `RmInitAdapter failed` | `prov.log`, `driver.log` | `PROVIDER` (`provider_gpu_unavailable`, `scripts/ci/vast_payload.py:35-43`) | Destroy, re-rent |
| Slow apt mirror, 13 kB/s | `box.log` | `PROVIDER` | `provision_box.sh` races mirrors; if it still stalls past T+30, destroy |
| Media hash or size mismatch | `install.log` `REFUSED: ... sha256` | `MEDIA` | Stop. Do not edit the pin on the box. Report the new `Content-Length` and hash to the owner |
| Install boot not finished at the timeout (`BOOT_TIMEOUT`) | `TIMEOUT after Ns` in `install.log`, screendump `timeout_*.png` | `UNATTRIBUTED` | Look at the screenshot: Setup's first page means the answer file was not read (measured defect, 2026-10-04, `V3_WINDOWS_DISCOVERY.md:143-145`), so `OPERATOR`; a progress bar frozen at one percentage on a box whose single-thread speed is low is `PROVIDER`. One install retry only if before T+55 |
| Install ends but `WIN_CHECK_DONE fail>0` | `installcheck_check.txt` | `OPERATOR` | Read which `WIN_*` line failed; not a kayfabe verdict (the install has no kf3 device, `win_vm.sh:556-559`) |
| Autopilot, a company tenant or a sign-in-to-organisation page at OOBE | screenshot | `OPERATOR` | The guest should have no route out (`restrict=on`); if it appears, the answer file or the network flag changed. Stop and report (`docs/howto/troubleshooting.md:75`) |
| Windows Update installs a display driver, or a driver install loops | `setupapi` tail, `pnputil /enum-drivers` | `OPERATOR` | WU is off by policy and there is no network (`autounattend.xml.in:72-74`); on the native-Windows path WU did deliver other drivers (`V3_CONSTANT_MATRIX_20261014.md:404`), so a hit here means the policy keys were lost |
| Driver install thrash: `pnputil /add-driver` skipped the disabled device, `DriverInfPath` still `display.inf`, or setup exits non-zero | `nv_install_*.txt` (`GSP_ON WARN`) | `OPERATOR` first | Re-run `nv-install` once; if the INF is still not bound, stop; not a kayfabe verdict |
| Code 43 and the kf3 log shows `GSP REFUSED fn76/0x2080121f` and no flags were exported | `arm_b1_qemu.log`, `enable_once.txt` | `OPERATOR` | The flags were missing (MEASURED 2026-10-27, lease 57882297, `w4_refused_set_no_flags.txt`). Fix the environment, new arm |
| Code 43 with the flags and the GSP keys read back as 1, host reference green | same | `KAYFABE-NEW` (or `KAYFABE-KNOWN` if it matches `docs/design/V3_WINDOWS_DISCOVERY.md` §4 or `traces/windows_code43_walls_20261007/`) | Capture the refused-control list and the last RPC ordinal; no wall-clearing loop (`AGENTS.md`, *Working rules*); max 2 arms |
| Boot to ssh over 20 min, guest ignores `shutdown /s` | `SSH_UP after Ns`, `stop.log` | follows Code 43 (`V3_WINDOWS_DISCOVERY.md:271-276`) | Classify by the Code 43 row |
| TDR storm: Display 4101 or `nvlddmkm` 153 triples every 20 to 30 s, RM unloads, bugcheck 0x113 or 0x141 | `tdr_counts.txt`, qemu log | `KAYFABE-KNOWN` on TU106 and GA106 (MEASURED 2026-10-27, leases 57882297 and 57908255, `traces/tur1_20261027/README.md:35`, `:99`); `KAYFABE-NEW` on other dies if the host reference is green | The bar is **not met**. Record the die, the timeline CSV and the `HOST-FENCE-OVERDUE` block if the completion probe is on |
| Ssh unreachable though the screenshot shows a desktop | `wait_ssh` log | `OPERATOR` until the connect timeout is raised (3.1 item 5) | Raise `ConnectTimeout`, retry; check `nvidia-smi` from the host first |
| Realize refusal: BAR1 sum too large or `fb-mb` above free VRAM | `arm_b1_qemu.log` first lines | `OPERATOR` (config) or the Vast BAR1 exception | Set `WINVM_KF3_FB_MB` lower; BAR1 refusals follow the exception rule above |
| GPU lock wait | `waiting for the GPU lock` | `OPERATOR` | Find the holder with `fuser`; never `kill` an unknown pid |
| QEMU gone with no guest shutdown (OOM, host reboot) | `dmesg`, `GUEST_PANICKED`, no `WINVM_RUN_END` | `PROVIDER` if the host `dmesg` shows an OOM kill or the box rebooted; else `UNATTRIBUTED` | A log with no `WINVM_RUN_END` belongs to a dead run |
| Host Xid during the window | `host_dmesg.txt` | `KAYFABE-NEW` only if the host reference is green and the Xid is not on the approved list (five Xid 31 records after the native suite, `ADA_RESULT.md`); otherwise `UNATTRIBUTED` | Stop the arm; do not start another on a box with a new Xid |
| Auto-destroy fired, or Vast destroyed the box | `autodestroy-<id>.log`, API list | `PROVIDER` if early, `OPERATOR` if the lane overran | Evidence not yet exported is lost: export after every stage from step 6 on |

## 8. Open questions for the owner

1. **Budget.** Is the 12 USD daily cap with the 8 USD floor in force for 2026-10-31? The ruling on record raised it "for 2026-10-25" (`docs/OWNER_RULINGS.md:1425`). How many attempts (leases) may the bar use?
2. **Duration.** Is 3 h hard, or may this lane have a 4 h exception with the checkpoints in section 2?
3. **GPU.** The best Windows records are on AD102 (about 1 USD/h class, INFERRED) and GB205; TU106 and GA106 are known to storm and are cheap. May the lane rent only AD104, AD102, AD106 or GB205 and refuse the cheap dies? No Windows run on AD104 exists on a rented box; its only record is the owner host (CLAIMED in `docs/design/V3_FAMILY_COVERAGE.md`).
4. **Image store.** Do you want the two-lease option (section 5.4) at all? If yes: a legal read of the evaluation terms, a restore subcommand, and a ruling on shipping per-VM secrets and the db key.
5. **Which flags make "production" for this bar?** The 19 in `p19.flags`, plus lazy observe, mirror and pointer lifecycle (the owner rule that correctness counts only with those on)? And is the flag-free default route required before the merge?
6. **What is "desktop reached"?** A signed-in `explorer.exe` (this plan, with autologon in a throwaway guest) or the sign-in screen with DWM running? Is registry autologon with a throwaway password acceptable on an untrusted box?
7. **Native reference.** Is a same-box Linux host reference enough for attribution, or must a failure also be reproduced on a native Windows lease (BW7 style, a separate rental)?
8. **Unmerged scripts.** May Codex use `windows_lane.sh` and `WINVM_INSTALL_CACHE` from draft PR #165 as they are, and may the `win_vm.sh` diffs in section 3 go in as one reviewed PR before the lease? Which kit revision is pinned?
9. **Per-VM identity.** May `win_vm.sh` get a per-VM `-uuid`, SMBIOS serial and MAC (3.2 item 5)? Does the Windows driver care that the GPU UUID changes on every QEMU restart?
10. **NVIDIA terms.** Is a direct download of the 580.88 installer on each rented box acceptable under the driver licence (listed as unreviewed in `docs/DOWNLOADS.md`)?
11. **Evidence.** May desktop PNGs of the evaluation Windows be committed (small, text-only otherwise)?
12. **Fix the docs?** `scripts/bench/windows/README.md:39` ("about an hour") overstates the recorded installs; the howto copies it (`docs/howto/windows-guest.md:68`). Update when the measured table is accepted.

## 9. UNSURE (not resolved by this research)

- Whether the 19-flag list is still the production list on `next`, and whether lazy observe, mirror and pointer lifecycle are on by default (only the flag names were checked, not their defaults beyond `KF3_DEFERRED_API` and `KF3_TRANSLATED_CE_RELAY`, which are `== "1"` opt-in at `crates/kf-qemu/src/chan.rs:1778` and `:1788`).
- The size of a sealed Windows `qcow2`, its compressed size, and a download rate from a private store (no recorded number).
- The create-to-running lag of a Vast KVM instance (no recorded number).
- The contents of the `win-gpu-image` repository's lock file (not read).
- Whether `--arm A` or `--arm B` is the right default: the README default is A, the recorded Code 0 holds used B (`arm.sh`), and the discovery run used A. This plan uses B.
- Whether `OverlayTestMode=5` is needed at all on LTSC: it was found on Windows 11 Pro, and the cause is not proven (`docs/WINDOWS_GUEST_SETUP.md:19`). The owner's bar names it, so the plan sets it.
- Whether the PR numbers #163 and #165 are still drafts (taken from the TUR1 report of 2026-10-27).
