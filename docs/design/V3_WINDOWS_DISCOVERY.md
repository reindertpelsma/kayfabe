# V3 — the first Windows guest on kf3 (discovery run, `vwin`)

**STATUS: LIVE, 2026-10-04.** Windows 11 boots on kf3 under Secure Boot with a TPM and the signed boot-display ROM. The NVIDIA 580.88 driver, forced into GSP mode, boots kf3's emulated GSP. It gets 141 commands into RM init, then stops at an unserved Windows-only control (`GR_GFX_POOL_QUERY_SIZE`) and shows Code 43 (§4). The harness that does all of this is `scripts/bench/windows/win_vm.sh`.

Owner, 2026-10-04, verbatim, in order: *"In qemu we can just enable 'secure boot' for a windows vm and
self sign the rom, I mean windows vms just work normally without complaint"* · *"For swtpm a secure
seed must be provided probably?"* · *"Yes but tpm should persist reboot though. And a hash of non secret
values isn't secure. So some secure seed must be stored right."* · *"Also disable bitlocker in the
windows guest, its useless for our vm."* · *"And load red hat virtio drivers. You know a script to later
boot windows + kayfabe is actually useful to not repeat all manual steps or a docker container, also
exposing a ssh to windows"* (recorded as `OWNER_RULINGS.md` §K on `origin/v3-owner-questions`).

Evidence: `traces/v3_windows/discovery_20261004/` (text logs and small PNGs only). The harness:
`scripts/bench/windows/` (`README.md` there is the user guide).

## 0. Results in one table

| step | result | evidence |
|---|---|---|
| provisioning | host driver 575.51.03 → **580.159.04 open**, QEMU 10.2.4 + kf3, Linux guests at 580.159.04 and 580.65.06 | §1.1 |
| Secure Boot chain | per-host kayfabe db key; per-VM vars (generated PK/KEK, Microsoft keys, kayfabe cert in `db`); kf-gop signed with `sbsign` and served through `gop-efi=` (`kf-gop SIGNED`) | §1.3, §2.1 |
| TPM | manufactured once from swtpm's CSPRNG; **the same EK across three QEMU processes and a guest reboot**, ready and owned throughout | §1.4, §2.2 |
| Windows install | Windows 11 Enterprise LTSC 2024 eval, unattended, 30 min; Secure Boot on; TPM 2.0; **BitLocker Fully Decrypted / Off from the first boot**; Red Hat virtio drivers (no balloon); OpenSSH keyed to the harness | §1.5 |
| boot on kf3 (no NVIDIA driver) | ★ TianoCore → Windows Boot Manager → lock screen on **kf-gop's BAR1 framebuffer at 1920×1080**; Microsoft Basic Display on `10DE:2504`; kf3 stays cold | §2.1 |
| NVIDIA 580.88, GSP forced | ★ nvlddmkm **boots kf3's emulated GSP** (FWSEC-FRTS, WPR2, booter, msgq); `bGspNocatEnabled=1`; fn 1 refused until C2 | §2.3 `nv1` |
| with C2 | ★ fn 1 accepted (Windows 580.88 = Linux 580.65.06); RM init runs 141 commands, then the guest **tears down after the unserved `GR_GFX_POOL_QUERY_SIZE`** → **Code 43** | §2.3 `nv2`, §4 |
| nvidia-smi / display takeover / CUDA | no driver to talk to / not reached / `cuInit` → 100 (no device) | §2.4 |
| BitLocker after the NVIDIA install | still Fully Decrypted / Off | §2.5 |
| Windows vs Linux | no new RPC function, no new alloc class; **14 Windows-only control ids, 9 of them refused** | §3 |
| harness | `scripts/bench/windows/win_vm.sh` (`install`, `run`, `ssh`, `stop`, …) used end to end; a fresh `run` after the install drove both stages | §1.5, README |

## 1. Setup

### 1.1 The box

| fact | value |
|---|---|
| host | vast 54071272 (`vwin`), Ubuntu 22.04.5, kernel 6.8.0-59, **a nested box** (`systemd-detect-virt` = kvm; Xeon E5-2673 v4, Broadwell; 19 vCPUs, 24 GiB) |
| GPU | RTX 3060, `10de:2504`, subsystem `3842:3656`, VBIOS 94.06.2F.00.95, 12 GiB |
| host driver | arrived 575.51.03 (closed), swapped by `provision_host_driver.sh` to **580.159.04 open** (`OPEN_MODULE=yes VERSION_MATCH=yes`, 2026-10-03 23:12 UTC) |
| bench tree | `provision_box.sh` (`KAYFABE_BRANCH=v3-windows`), `provision_host_driver.sh`, `provision_bench_tree.sh` (QEMU 10.2.4 + kf3, Linux guest at 580.159.04): all rc=0, 14 minutes in total |
| Linux baseline guest | `stage_guest_driver.sh 580.65.06` + `stage_fat_guest.sh 580.65.06` (modinfo and libcuda both 580.65.06) |
| Windows packages | `ovmf` 2022.02-3ubuntu0.22.04.6, `swtpm`/`swtpm-tools` 0.6.3, `python3-virt-firmware` 24.1.1, `sbsigntool` 0.9.4, `xorriso`, `netpbm`, `p7zip-full`, `gcc-mingw-w64-x86-64` (all distro packages) |

Every kf3 binary is per revision (`/workspace/bench/kf3-bins/<rev>/`). Each run below names its revision.

### 1.2 Code changes on `v3-windows`

| id | change | commit |
|---|---|---|
| C1 | `build_kf3.sh`: `--enable-tpm` (and `--enable-slirp` for the harness's localhost-only ssh) | `494bfeaa`, `f40cd822` |
| C3 | log only: fn 1's `guestDriverVersion`/`guestVersion`/`guestTitle`/`guestClNum`/vGPU pair; fn 72's `bGspNocatEnabled`; `KF3_RPC_TRACE=1` = one line per command the chain answers (id + result) | `f0510a9a`, `d46240c0` |
| C4 | a **signed** GOP driver supplied at run time: `kf_oprom::pe::signed_twin_of`, `kf_gop_image::pack_kf_gop_signed`, bin `kf-gop-export`, kf3 property `gop-efi=` (KF3 ABI 12) | `8414ccbc`, `f8ee966a` |
| C5 | kf-disp display tables for guest 580.65.06 (re-derived; identical rows to 580.159.04's) | `20471c65` |
| C2 (part 1) | each driver-matrix tag's Windows twin, read from its own `nvBldVer.h` (`tools/drivermatrix/windows_twins.py` → `traces/driver_matrix/windows_twins.tsv`, kf-abi `windows_twin`) | `0eabbdde` |
| C2 (part 2) | fn 1 keys a Windows guest as its Linux twin: matched on the observed identity (`guestDriverVersion` + `guestVersion` against the tag's Windows block; `guestClNum` 0 or equal), used by `ReselectAtFn1` and the chain's version check | `f429b2be`, `1b575ebf` |
| C6 | `scripts/bench/windows/`: `win_vm.sh` and its guest scripts, `rpc_diff.py`, the mingw CUDA ladder | `f40cd822` … |

### 1.3 The Secure Boot chain

1. **The key, per host.** `win_vm.sh install` generates an RSA-3072 db key and a self-signed certificate
   (`CN=kayfabe GOP db key (<host> <date>)`) into `$WINVM_ROOT/secureboot/` (0700). The private key
   never leaves the box and never enters cargo.
2. **The vars, per VM, created once.** `virt-fw-vars --enroll-generate "kayfabe VM kfwin PK/KEK"
   --add-db <guid> db.crt --secure-boot` over `OVMF_VARS_4M.fd`. Result
   (`traces/v3_windows/discovery_20261004/install/ovmf_vars_print.txt`):
   - PK is the generated kayfabe cert.
   - KEK is the kayfabe cert plus Microsoft KEK CA 2011.
   - `db` is Windows Production PCA 2011, Microsoft UEFI CA 2011 and the kayfabe GOP db cert.
   - `SecureBootEnable` is ON.
   - ⊘ Deviation from the runbook: jammy's distro virt-firmware (24.1.1) was used, not pip's 26.9, so
     no 2023 CA was enrolled at creation. The LTSC 2024 media is signed by Windows Production PCA 2011,
     so this was enough to boot.
3. ★ **The guest changed the vars.** Before Windows Setup got past its first page, `db` gained
   Windows UEFI CA 2023, Microsoft UEFI CA 2023 and Microsoft Option ROM UEFI CA 2023, beside the 2011
   CAs and the kayfabe cert; KEK and `dbx` changed too. Every later check listed them. So the per-VM
   vars file is live state that the guest changes, which is one more reason it is created once and
   never regenerated. ⚠ The KEK part is unexplained: see §2.6.
4. **The signed ROM.** `win_vm.sh sign` builds `kf-gop-export` from the same checkout as the kf3
   binary, writes the embedded driver, signs it with `sbsign`, and checks it with `sbverify`. kf3 gets
   `gop-efi=<signed.efi>`. At realize, kf3 accepts the file only as the embedded driver plus an
   Authenticode table (`signed_twin_of`), and its log line says `kf-gop SIGNED (cert table …)`.
5. ⚠ **What this does not prove.** Stock OVMF's option-ROM policy is 0x00: it runs any option ROM,
   signed or not. So this run shows that the signed ROM is served and that Windows boots with Secure
   Boot on. Enforcement of the signature is proven separately, by F1 on the deny-policy OVMF
   (`kf-oprom/src/pack.rs` ★ TESTED note, snakeoil keys; §5 lists re-running F1 with this run's
   per-host key).

### 1.4 The TPM (owner §K: "the state file is the secret")

- **Manufactured once.** `swtpm_setup --tpm2 --create-ek-cert --create-platform-cert --lock-nvram
  --pcr-banks sha256 --not-overwrite` ran at 2026-10-03 23:37 UTC
  (`install/swtpm_setup.log`).
  - Seeds come from swtpm's CSPRNG. There is no `--vmid` and nothing derived from a name.
  - The state is 0600 in a 0700 directory.
  - `win_vm.sh` refuses to manufacture over an existing `tpm2-00.permall`. The second install run
    (after a harness fix) logged `TPM_EXISTS … reused`.
- **Every QEMU start uses the same state:** `swtpm socket --tpmstate dir=… --terminate`, with a fresh
  swtpm process per QEMU.
- ⊘ **Deviations.** swtpm 0.6.3 has no `lock` sub-option. Instead, a per-VM `flock` and a live-pid
  check stop two swtpm processes from serving one state. jammy's swtpm runs under an enforced AppArmor
  profile scoped to libvirt paths, so the harness adds one rule (`$WINVM_ROOT/** rwk,`) to the profile's
  local include, once.
- **Persistence, measured:** see §2.2.

### 1.5 The Windows install (no kf3; std VGA; Secure Boot + TPM on)

**Media.** Windows 11 Enterprise LTSC 2024 evaluation (26100.1742), 5 112 850 432 bytes, sha256
`67cec586…`, from Microsoft's fwlink. virtio-win 0.1.302 from its versioned archive. Win32-OpenSSH
10.0.0.0p2 (MSI). All are pinned by sha256 in `win_vm.sh`.

**Timeline.** `win_vm.sh install` at `1c479f30`, 2026-10-03 23:44 → 2026-10-04 00:17 UTC:
- install boot: 30 min, ended `guest-shutdown`;
- first logon: `KF_FIRSTLOGON OK`;
- check boot: ssh up 20 s after start;
- then sealed. `install/install.log` has every step's `START`/`EXIT`.

**Check boot result.** `install/installcheck_check.txt` is `WIN_CHECK_DONE fail=0`:

| item | result |
|---|---|
| edition | Windows 11 Enterprise LTSC Evaluation, build 26100 |
| Secure Boot | `Confirm-SecureBootUEFI` = True; `db` has the kayfabe cert |
| TPM | 2.0; present, ready, enabled, activated, owned |
| BitLocker | Fully Decrypted, 0.0 %, Protection Off, `PreventDeviceEncryption=1`. The prevention held, so no `manage-bde -off` was needed. |
| virtio | viostor, netkvm, vioser, viorng, pvpanic, fwcfg, smbus in the driver store and bound; no balloon, no viomem |
| PnP | no problem devices |
| sshd, QEMU-GA | Running |
| Windows Update | off by policy |
| VBS | 0 |
| hibernation, Fast Startup | off |
| Smart App Control | off |

**Two harness defects found on the way.** Both were fixed and are recorded in their commits:
- `step()` ran its body under `|| rc=$?`, where bash ignores `set -e`. A failed answer-file render
  therefore read `EXIT rc=0`, and Setup stopped at its first page.
- The vars check grepped a print mode that lists no certificate subjects.

## 2. Results

Every kf3 run below names the revision it ran at. All are on vwin's RTX 3060 with host driver 580.159.04
(open), on 2026-10-04.

### 2.1 Stage 1: Windows boots on kf3 with the signed GOP ROM (`basic1`, kf3 `b98bdbec`)

**Configuration.** `win_vm.sh run --tag basic1 --overlay s1 --guest-driver 580.65.06`, which gives
`kf3-gpu,…,display=on,gop=on,gop-efi=…/kf-gop.b98bdbec.signed.efi`, `-vga none`, arm A (no Hyper-V
enlightenments).

**kf3 at realize:** `boot display ON — option ROM 11776 bytes (10de:2504, KFGP BAR1 +0x0, 1920x1080 pitch
7680, G = 0x7f0000) … kf-gop SIGNED (cert table 2008 bytes at +0x2400, WIN_CERTIFICATE 2008 bytes; …)`.

**On screen** (`basic1/shots/`):
- `basic1_boot_tianocore_bootmgr.png`: kf-gop's 1920×1080 framebuffer shows TianoCore and
  `BdsDxe: starting Boot0004 "Windows Boot Manager"` with the Windows spinner. bootmgr drew on kf3's
  GOP with Secure Boot on.
- `basic1_lockscreen_basic_display.png`: the Windows lock screen at 1920×1080.

**In the guest** (`basic1/check_*.txt`; `basic1/evidence/`):
- `Microsoft Basic Display Adapter [PCI\VEN_10DE&DEV_2504&SUBSYS_36563842&REV_A1…] status=OK`. Windows
  bound its inbox Basic Display driver to kf3 over the GOP framebuffer.
- Every other `WIN_*` line passes as at install.
- ssh was up about 150 s after the QEMU start.

**kf3 stayed cold** the whole time: `phase=Cold trapped=0`. Without the NVIDIA driver, Windows never wrote
BAR0; only the boot framebuffer was used.

### 2.2 TPM and Secure Boot persist across a guest reboot and a QEMU restart

The guest ran `shutdown /r /t 0` (`basic1/basic1_run.log`):

```
WINVM_BOOT_END boot=1 rc=0 reason=guest-reset …
guest rebooted: kf3 has no reset path, so a FRESH QEMU (same vars, same TPM state)
QEMU_START tag=basic1 boot=2 …
```

The TPM's endorsement key (`Get-TpmEndorsementKeyInfo`, SHA-256 of the public key), as recorded in
`basic1/tpm_ek_history.txt`, was identical in three QEMU processes and across two kf3 binaries:

| when (UTC) | QEMU process | EK `PublicKeyHash` | TPM |
|---|---|---|---|
| 00:17:05 | install check boot (`1c479f30`, std VGA) | `35dfefab…845e79cfe` | present, ready, owned |
| 00:33:38 | basic1 boot 1 (`b98bdbec`, kf3) | `35dfefab…845e79cfe` | present, ready, owned |
| 00:41:26 | basic1 boot 2, after the guest reboot | `35dfefab…845e79cfe` | present, ready, owned |

`Confirm-SecureBootUEFI` was True and the kayfabe cert was in `db` at every check. BitLocker stayed
FullyDecrypted / Off.

### 2.3 Stage 2: NVIDIA 580.88, GSP forced on (`nv1`, `nv2`)

**Install** (`nv1`, kf3 `b98bdbec`, fresh overlay `nv1`):
- `win_vm.sh nv-install` copied `Display.Driver` (2.6 GB) into the guest. Over slirp this took
  18 minutes (about 2.4 MB/s); §5 proposes an ISO instead.
- `gsp_on.ps1` then did the rest (`nv1/nv_install_*.txt`):
  1. Found `PCI\VEN_10DE&DEV_2504…`, driven by `display.inf` (Basic Display).
  2. `nvlddmkm.sys` signature: Valid, NVIDIA Corporation.
  3. Disabled the device.
  4. `pnputil /add-driver nv_dispi.inf /install`: published as `oem7.inf` and **installed on the disabled
     device**. `DriverInfPath` became `oem7.inf`, version 32.0.15.8088, so the runbook's fallback was not
     needed.
  5. Wrote `EnableGpuFirmware=1` into the class key `{4d36e968…}\0001` and into
     `Services\nvlddmkm`, and read both back as 1.
  6. Enabled the device. nvlddmkm then started for the first time on a kf3 that had never seen an RM.

#### `nv1`: the driver runs the whole GSP boot, then fn 1 is refused

From `nv1/nv1_b1_qemu.trimmed.log`, in order:

```
kf3: GSP phase Cold -> ProtectedRegionUp
kf3: GSP WPR2 up at the guest's FWSEC-FRTS command: frts_offset=0x1ffe00000 …
kf-gsp: fn 72 GSP_SET_SYSTEM_INFO seq=0 936 bytes: bGspNocatEnabled=1 at +929 (driver 580.65.06's layout; …)
kf3: GSP rpc SetRegistry seq=0
kf3: GSP phase ProtectedRegionUp -> Booted
kf-rm: fn 1 SET_GUEST_SYSTEM_INFO: the guest says guestDriverVersion="580.88" guestVersion="r580_78-7"
       guestTitle="DVSReal r580_78 580.88 DVS-Applications" guestClNum=0 vgx=0x2b.0x13; this device answers as driver 580.65.06
kf-rm: SET_GUEST_SYSTEM_INFO refused: the guest driver says it is "580.88" but this device's layouts were selected for 580.65.06 …
kf3: GSP REFUSED fn1=0x56 (SET_GUEST_SYSTEM_INFO)
```

The Windows side: `NVIDIA GeForce RTX 3060`, `CM_PROB_FAILED_POST_START` (Code 43).

Four things this establishes:
1. ★ **`EnableGpuFirmware=1` in the class key forces a stock GeForce driver into GSP mode on Ampere.**
   A message queue appeared and fn 72 arrived, so this was not monolithic RM. This is the Ampere data
   point that `THE_WINDOWS_AXIS.md` §1 lacked (it had Turing and Ada with GSP off by default). The value
   was written before nvlddmkm's first start, so what the default would have been on this SKU was not
   tested.
2. ★ **The GSP boot path is OS-agnostic, as §6.3 there predicted.** FWSEC-FRTS, WPR2, the booter and
   the message queue all worked against kf3's emulation for a Windows RM, with no change.
3. ★ **`bGspNocatEnabled=1` at fn 72**, which is §7's Windows detector, and the 936-byte
   `GspSystemInfo` matches 580.65.06's layout.
4. ⊘ **The runbook's C2 premise was half right.** The name is `580.88`, as inferred from the binary's
   strings. But `guestClNum` is **0**, not 36308443: this is a `DVSReal` build, without the
   buildmeister define, whose `NV_BUILD_CHANGELIST_NUM` falls back to `NV_BUILD_CL`. The Linux `.run`
   builds also send 0 (`lin_b1t`: `guestTitle="Private …" guestClNum=0`). So a changelist check on the
   wire could never pass. C2 (`f429b2be`) matches on the two strings instead (`580.88`, `r580_78-7`),
   against 580.65.06's own `nvBldVer.h` Windows block, whose two changelists are equal.

#### `nv2`: with C2, RM init gets 141 commands in, then the guest tears down (kf3 `f429b2be`)

Same overlay, a fresh QEMU, nvlddmkm loading at boot (`nv2/nv2_b1_qemu.trimmed.log`):

```
kf-rm: the guest is Windows 580.88 (r580_78-7), the Windows build of Linux 580.65.06 (one changelist, 36308443): keyed as 580.65.06
kf3: GSP phase Booted -> Running
… GET_GSP_STATIC_INFO, 23 allocs, 90 controls (33 refused: 31 unserved, 2 by kf3's encoders), one kernel GR channel, frees …
kf-rm: rpc-trace fn=76 RmControl cmd=0x2080121f … result=none        ← NV2080_CTRL_CMD_GR_GFX_POOL_QUERY_SIZE
kf3: GSP REFUSED fn76/0x2080121f=0x56
kf-rm: rpc-trace fn=10 Free … (×4), INTERNAL_BUS_FLUSH_WITH_SYSMEMBAR, L2_INVALIDATE_EVICT
kf3: … the guest wrote NV_PBUS_BAR1_BLOCK = 0x0 (MODE PHYSICAL) … — the guest's RM gave BAR1 up
… more frees, UPDATE_BAR_PDE (BAR2 root cleared) …
kf3: GSP fn 47 UNLOADING_GUEST_DRIVER … ; GSP phase Running -> Suspending -> Halted
```

**Windows side** (`nv2/evidence/`):
- `NVIDIA GeForce RTX 3060`, `DEVPKEY_Device_ProblemCode 43`, `ProblemStatus 0`, `oem7.inf`
  32.0.15.8088.
- Basic Display does not take over a device whose driver failed, so the screen stayed on the boot
  framebuffer.
- Boot to ssh took about 20 minutes, against about 2.5 minutes without the NVIDIA driver. Windows wrote
  a `LiveKernelReports\PoW32kWatchdog-20261004-0158.dmp` (the Win32k power watchdog) 4.5 minutes into
  that boot (`nv2/evidence/dumps.txt`). The dump stays in the guest and is not committed.
- No `nvlddmkm` event in the System log; kf3's log is the only place the failure is visible.
- Host: no Xid, empty host dmesg delta.
- Shutdown: the guest did not power off within 5 minutes of `shutdown /s`, so the harness ended QEMU
  with QMP `quit` (`reason=host-qmp-quit`).

### 2.4 nvidia-smi, display takeover, CUDA

| check | result |
|---|---|
| `nvidia-smi.exe -q` / `-L` | `NVIDIA-SMI has failed because it couldn't communicate with the NVIDIA driver` (`nv2/evidence/nvidia_smi_q.txt`) |
| display takeover | none: no head was armed and no window scanned. The screen stays on the GOP framebuffer. |
| CUDA (`cup2.exe`, `cup3.exe`, mingw, `scripts/bench/windows/cuda/`) | `nvcuda.dll` present; `FAIL cuInit(0) -> no CUDA-capable device is detected (100)` |

### 2.5 BitLocker after the NVIDIA install (owner §K)

`nv2/evidence/bitlocker.txt` shows `FullyDecrypted`, `Protection Off`, `0.0%`, and
`PreventDeviceEncryption 0x1`. The prevention held through the driver install and four QEMU restarts.

### 2.6 ⚠ An open Secure Boot question: KEK changed

Between the install's first QEMU start (23:37, Windows media booted to Setup's first page) and its
second (23:44), the VM's vars gained entries in three variables (`install/install.log`,
`install/ovmf_vars_print.txt`, `install/ovmf_vars_after_windows.txt`):

| variable | gained |
|---|---|
| `db` | Windows UEFI CA 2023, Microsoft UEFI CA 2023, Microsoft Option ROM UEFI CA 2023 |
| `KEK` | Microsoft Corporation KEK 2K CA 2023 (2343 → 3849 bytes) |
| `dbx` | 220 hashes (76 → 10664 bytes) |

The `db` and `dbx` additions are what Microsoft's KEK CA 2011, which is in this KEK, may sign. A KEK
append must be signed by the **PK**, and this VM's PK is the generated `CN=kayfabe VM kfwin PK/KEK`,
whose key lives only in virt-fw-vars' run. The only code that ran in that window was OVMF and the Windows
media's boot manager or WinPE. ⊘ **How that write was authorized is unexplained.**

This does not block the run. But §K's chain assumes `db` and KEK change only under keys we hold, so this
must be explained before we rely on it: does this OVMF build enforce authenticated writes to KEK, and was
the PK really enrolled when Windows first ran?

## 3. Windows vs Linux: what the Windows driver used that the Linux guest never does

**Baseline:** Linux guest 580.65.06 on the same box with the same kf3 flags (`display=on,gop=on,
guest-driver=580.65.06`), kf3 `b98bdbec`, with `KF3_RPC_TRACE=1` (`linux_baseline/`). The display lane
passed (boot handoff `black_frames=0`), and the CUDA ladder passed:

| rung | result |
|---|---|
| cup2 | `CE rv=0xabcd1234` |
| cup3 | `CUP3_VAL=43` |
| cup8 | `CUP8_BAD=0 CUP8_MAXERR=0` |

**Windows:** `nv2` (kf3 `f429b2be` = `b98bdbec` + C2 + a harness lock fix). Full table:
`windows_vs_linux.md`, from `scripts/bench/windows/rpc_diff.py`. Ranked by kind:

| kind | Windows | Linux | Windows-only |
|---|---|---|---|
| fn 72 `bGspNocatEnabled` | **1** | 0 | the Windows detector (THE_WINDOWS_AXIS §7) |
| fn 1 identity | `580.88` / `r580_78-7` / `DVSReal …` / CL 0 | `580.65.06` / `rel/gpu_drv/r580/r580_78-179` / `Private …` / CL 0 | keyed to 580.65.06 by C2 |
| RPC functions | the same 9 as Linux | 9 | **none**: no new RPC function |
| first divergence (ordered) | index 2: the guest allocates its own client (`0x0000`/`0x0080`/`0x2080`/`0x2081`) first | index 2: `INTERNAL_GPU_GET_CHIP_INFO` on the internal client | ordering only |
| RM_ALLOC classes | 8 | 20 | **none** |
| GSP_RM_CONTROL ids | 72 | 124 | **14** (below) |
| shared ids answered differently | 3 | — | `GPU_GET_INFO_V2`, `EVENT_SET_NOTIFICATION`, `BUS_GET_INFO_V2` |
| kernel GR channel at init (`0xc56f`, `GPU_PROMOTE_CTX` refused, "kernel GR is P7") | yes | yes | shared, survived on Linux |
| `UPDATE_PDE_2` / `RESERVE_ENTRIES` / doorbell / paging-buffer traffic | not reached | — | init never got that far |

**The 14 Windows-only control ids, with what kayfabe answered.** "Unserved" means no link answered and
the state machine's default refusal (`0x56`) went to the guest.

| id | name (ogkm 580 SDK) | kayfabe answered |
|---|---|---|
| `0x2080121f` | `NV2080_CTRL_CMD_GR_GFX_POOL_QUERY_SIZE` | **unserved → 0x56**; the last command before the teardown |
| `0x20801303` | `NV2080_CTRL_CMD_FB_GET_INFO_V2` | 0x56: kf3's encoder refused `UnmeasuredIndex { index: 1 }` |
| `0x00800292` | `NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2` | unserved → 0x56 |
| `0x00801306` | `NV0080_CTRL_CMD_FB_GET_COMPBIT_STORE_INFO` | unserved → 0x56 |
| `0x20800160` | `NV2080_CTRL_CMD_GPU_GET_VPR_CAPS` | unserved → 0x56 |
| `0x20800173` | `NV2080_CTRL_CMD_GPU_QUERY_FUNCTION_STATUS` | unserved → 0x56 |
| `0x20800aaf` | `NV2080_CTRL_CMD_INTERNAL_GET_ENABLED_SEC2_CLASSES` | unserved → 0x56 |
| `0x20800ab8` | `NV2080_CTRL_CMD_INTERNAL_GET_PCIE_P2P_CAPS` | unserved → 0x56 |
| `0x00801707` | `NV0080_CTRL_CMD_FIFO_GET_ENGINE_CONTEXT_PROPERTIES` | 0x0 |
| `0x00801b02` | `NV0080_CTRL_CMD_MSENC_GET_CAPS_V2` | 0x0 |
| `0x00801c02` | `NV0080_CTRL_CMD_BSP_GET_CAPS_V2` | 0x0 |
| `0x20801315` | `NV2080_CTRL_CMD_FB_GET_GPU_CACHE_INFO` | 0x0 |
| `0x20808159` | not in the public SDK headers | 0x0 |
| `0x20809001` | not in the public SDK headers | 0x0 |

**Also Windows-only:** `NV2080_CTRL_CMD_BUS_GET_INFO_V2`'s second call, which asked for info index 24
(`UnmeasuredIndex { index: 24 }` → 0x56). Linux asked only for indices kf3 answers.

## 4. Blockers, ranked

1. **`NV2080_CTRL_CMD_GR_GFX_POOL_QUERY_SIZE` is unserved.** It is the last command before the guest
   frees everything and unloads.
   - The export table has it as flag `0x40`, compiled out of the Linux builds
     (`ogkm-580.159.04 src/nvidia/generated/g_subdevice_nvoc.c:5500-5512`). The header says
     *"queries size parameters for a request maximum graphics preemption pool size. It is only
     available to kernel callers"* (`ctrl2080gr.h:1298-1333`).
   - This is the WDDM KMD sizing the GfxP (graphics preemption) pool. On real hardware the Windows GSP
     firmware answers it.
   - ⚠ **Neither kf3 nor the Linux host RM has an answer.** The host build compiles the control out, so
     "ask the host" is not available. Serving it means **authoring** `ctrlStructSize`/`poolSize`/
     `slotStride` from the GR's preemption buffer sizes. That needs a source of truth (the Windows GSP
     firmware's behaviour, or a real Windows box's answer); it is not a one-line fix.
   - ⊘ Inference, not measurement: "the last refusal before the frees is the fatal one" is an ordering
     argument from one run. Serving this control and watching the next run is the test.
2. **Eight more Windows-only refusals** (§3), each possibly fatal after (1) is served. The two with
   kayfabe-side causes are cheap to measure:
   - `FB_GET_INFO_V2` index 1 and `BUS_GET_INFO_V2` index 24: indices kf3 refuses as unmeasured.
   - `GPU_GET_CLASSLIST_V2`: the class list Windows reads; Linux reads the V1 form.
3. **The boot cost of a failed driver.** About 20 minutes from boot to ssh when nvlddmkm fails. This
   affects every iteration of this lane, not correctness.
4. **Not reached, still the big ones** (THE_WINDOWS_AXIS §2–3):
   - how WDDM's page-table updates reach the GPU (`UPDATE_PDE_2`, paging buffers);
   - doorbells rung from the KMD;
   - the 2 s TDR budget.

   None of them was exercised. Init stops before the first user channel.

## 5. Next steps

1. **Serve `GR_GFX_POOL_QUERY_SIZE` from an authored rule, by name.** Derive the sizes from the GR
   context-switch and preemption buffer sizes kf3 already answers (`INTERNAL_STATIC_KGR_*`). Or, better,
   measure the answer once on a real Windows + RTX 3060 with `EnableGpuFirmware=1`. Then rerun `nv2` as is
   (`win_vm.sh run --overlay nv1 --guest-driver 580.65.06`).
2. **Measure the two refused info indices and `GPU_GET_CLASSLIST_V2`** (§4.2) against the host,
   following the driver-matrix rule.
3. **Explain the KEK write (§2.6) before §K's chain is relied on.**
   - Re-run F1 (`scripts/display/build_ovmf_deny.sh`, `gop_standin.sh sb_deny_*`) with this host's db key.
     That proves the signature is enforced; this run proved only that it is served.
4. **Harness:**
   - hand the driver to the guest as an ISO (18 minutes over slirp today);
   - give `nv-install` an option to do its install offline on a powered-off overlay;
   - make `stop` wait longer when the NVIDIA driver failed (5 minutes was not enough).
5. **Arm B** (Hyper-V enlightenments) once init completes. Arm A ran here; RM takes Hyper-V branches with
   `Microsoft Hv`.
6. **Second SKU and driver:** the same flow with 582.53 (the 580.159.04 twin by name, with a different
   changelist; refused by C2 by design until the owner rules) and a non-Ampere card.

## 6. Deviations from the runbook, and harness defects found

- **virt-firmware:** jammy's distro 24.1.1, not pip's 26.9 (the task required distro packages). No
  2023 CA was enrolled at creation; Windows added them (§2.6).
- **swtpm 0.6.3:** has no `lock` sub-option (replaced by a per-VM flock and a pid check). Its AppArmor
  profile needed one local rule.
- **Networking:** QEMU user networking with `restrict=on` and a `hostfwd` bound to 127.0.0.1, not a tap.
  This needed `--enable-slirp` in `build_kf3.sh`. The guest has no route out.
- **C2:** matched on the two fn-1 strings, not on the changelist, because `guestClNum` is 0 (§2.3, 4th
  point).
- **CL checks in `nvBldVer.h`:** 10 of the 29 matrix tags share one changelist between their Windows and
  Linux builds (`traces/driver_matrix/windows_twins.tsv`).
- **Harness defects, each fixed and committed:**
  - `step()` reported a dead step as `rc=0` (`1c479f30`);
  - the vars check read a print mode without subjects (`c599ee6d`);
  - background helpers held the lock fds after a run ended (`dcdac15a`).

## 7. What this run does not show

- That the GOP **signature** is enforced. Stock OVMF option-ROM policy is 0x00; see §1.3.
- That GSP is **off by default** on Ampere GeForce under Windows. The value was forced before the first
  start.
- Anything past RM init: no user channel, no doorbell, no page-table write, no TDR.
- Byte identity of the Windows and Linux GSP firmware. `.fwversion` differs (`580.65.05` vs
  `580.65.06`); kf3 does not run the firmware, so this did not matter here.
- Results on bare metal or a non-nested box. vwin is a nested VM (Broadwell, KVM in KVM).

