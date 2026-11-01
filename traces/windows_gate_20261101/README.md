# Windows gate: the first real lease, 2026-11-01

STATUS: LIVE, 2026-11-01. Evidence of the first hardware runs of `scripts/bench/box/windows_gate.sh` (design:
`docs/design/V3_WINDOWS_GATE.md`). One Vast lease, four gate attempts on one installed Windows image, the last one a
`WINDOWS_GATE_PASS`. Labels: MEASURED = a line in a log in this directory, named with its file; INFERRED = reasoning.

## The lease

| | |
|---|---|
| lease | Vast instance 58592558 (own id), offer 52153651, Pennsylvania US, 0.502 USD/h, created 2026-10-31T23:57Z, destroyed 2026-11-01T02:13Z (about 2 h 16 min, about 1.1 USD from the account credit 41.82 to 40.73) |
| box | RTX 4090 (AD102, 24 GB), a guest on an AMD EPYC 7502 host with 15 vCPU (`/proc/cpuinfo` at the end of the lease, not saved), 98 GB RAM, 146 GB root disk, host BAR1 256 MiB (`g4/hostref_before.txt`), image `docker.io/vastai/kvm:ubuntu_cli_22.04-2025-05-16` |
| host driver | stock 575.51.03 on arrival (seen at the first ssh, not saved), then the open 595.91.07 from `provision_host_driver.sh` (`g4/hostref_before.txt`) |
| revisions | lane provisioned from branch `claude/windows-gate-20261101` at `b6ad020e`; gate attempts at `ed37318e` (g1), `d18032f3` (g2), `13926b0f` (g3), `a5071c24` (g4); the code of `a5071c24` is the code of the PR head except for docs and these traces |
| teardown | `vastai destroy instance 58592558 -y`, then `vastai show instances-v1 --raw` listed 0 instances |

## Timings (MEASURED 2026-11-01, lease 58592558, `lane/winlane.log` and `g4_windows.log`)

`lane/winlane.log` (`windows_lane.sh`, branch `b6ad020e`): `WINLANE_TIMING total=2596s box=194s guest_run=36s driver=195s tree=298s kf3=14s prereqs=21s install_windows=1838s`, `WINLANE_EXIT rc=0`. The install is `lane/install.log`: install boot 00:17:02Z to 00:42:55Z (25.9 min), check boot, seal at 00:47:05Z; the four pinned media passed their sha256 (`MEDIUM_OK` lines).

`g4_windows.log`: `WINDOWS_GATE_TIMING total=1621s preflight=0s hostref_before=1s prereqs=0s kf3_binary=14s install=0s probes_build=5s boot1=161s check_kf3=53s nv_install=327s desktop_prep=18s reboot=167s desktop=41s enable=92s smi_probes=86s idle=326s post_idle=106s collect_stop=198s`.

So one lease took about 43 min to a sealed install plus about 27 min for the gate itself; the rest of the 2 h 16 min is the three earlier gate attempts and the debugging between them.

## The verdict (g4, `g4_windows.log`)

```
WINDOWS_GATE_PASS rev=a5071c24 gpu="NVIDIA GeForce RTX 4090" idle_s=300 hostref=green/green secs=1620 install=once driver=580.88 gsp=forced code=0 probes=cup8,d3d11,x2 resets=0 known_live_dumps_193=2 strict=0 enable_utc=2026-11-01T01:57:00Z
WINDOWS_GATE_EXIT rc=0 2026-11-01T02:10:37+00:00
```

What the g4 log shows, step by step (all MEASURED 2026-11-01 in gate run g4, rev `a5071c24`, `g4_windows.log` and `g4/`):

- host reference before and after green: `GATE_HOSTREF before green cup8=PASS xid_count=0`, `GATE_HOSTREF after green cup8=PASS xid_count=0 xid_delta=0` (`g4/hostref_*.txt`, `hostref_*_cup8.txt`: `CUP8 VERDICT: PASS`).
- guest: `Microsoft Windows 11 Enterprise LTSC Evaluation`, build 26100 (`g4/guest_collect/os.txt`), Secure Boot true, TPM present and ready (`guest_collect/secureboot_tpm.txt`), `check.ps1 --kf3` `WIN_CHECK_DONE fail=0` before and after the driver (`check_before_driver.txt`, `check_after_driver.txt`).
- GSP forced: `EnableGpuFirmware=1` read back in the class key `...\{4d36e968-...}\0001` and in `Services\nvlddmkm` (`nv_install.txt`, `guest_collect/nvlddmkm_reg.txt`); autologon and `OverlayTestMode=5` read back (`desktop_prep.txt`).
- desktop: `GATE_DESKTOP user='KFWIN\kf' dwm=1 explorer=1`, console session active (`desktop_state.txt`).
- the one-time enable at 01:57:00Z (guest clock) took the adapter from `code=22` to `code=0`, driver 32.0.15.8088 (`enable_once.txt`, `display_after_enable.txt`); `nvidia-smi` shows `Driver Version : 580.88`, `Product Architecture : Ada Lovelace`, `GSP Firmware Version : 580.88`, WDDM (`nvidia_smi_q_guest.txt`).
- graded probes, before and after the 300 s idle, all `rc=0` (`probe_cup8*.txt`: `CUP8 RESULT N=2048 bad=0 maxerr=0 ... PASS`; `probe_d3d11*.txt`: `KFDX RESULT OK api=d3d11`, compute and draw read back); reported, not graded: `cup3` (`KERNEL rv=43 want=43 -> PASS`) and `kf_dxprobe d3d12` (`KFDX RESULT OK api=d3d12`).
- reset counters since the enable (`tdr_counts.txt`): `display4101=0 nvlddmkm153=0 kernelpower41=0 eventlog6008=0 bugcheck1001=0 livekernelevent_other=0 livekernelevent193=2 strict=0 total=0`.
- the kf3 log (`gate_g4_b2_qemu.log.gz`, summary in `gsp_signals.txt`) records no refusal of `0x2080121f` (`refused_0x2080121f=0`; the control was served once, `served_0x2080121f=1`), and 0 host Xid in both boots.
- graceful stop: `stop.log`, no QEMU left.

## What is NOT clean (read before relying on the PASS)

1. **WATCHDOG live dumps, LiveKernelEvent 193.** Every run that reached Code 0 shows two `LiveKernelEvent 193` WER reports and one 1.4 GB `WATCHDOG-*.dmp` in `C:\Windows\LiveKernelReports` about 1 minute after the enable (g4: dump at 01:58:06Z, `g4/guest_collect/dumps.txt`; g3: `WATCHDOG-20261101-0129.dmp` at 01:29:57Z, `g3_windows.log`; g2: 01:04:59Z by a manual query during the debugging, not saved here). No TDR, no reset and no probe failure came with them. The gate counts them and reports them (`known_live_dumps_193=2`) but does not let them fail the gate unless `KF_WINGATE_STRICT_LKE=1`; with the strict switch these runs would FAIL. The same event was already on record for an Ada run (`docs/design/V3_WINDOWS_ON_VAST.md` section 6.4, lease 57193877) and for GB205 (`docs/design/V3_FAMILY_COVERAGE.md`). Its cause is not known and nothing here proves the guest is healthy beyond the 300 s window, the probes and the zero reset counters. This is a decision for the owner: gate on it (the gate then fails today) or keep it a reported open defect (the gate then passes).
2. **`gpu-uuid=auto` does not realize on the stock 595.91.07 host** (`g1_windows.log`; `kf3: realize refused: gpu-uuid=auto needs the host GPU's UUID and the host RM did not answer NV2080_CTRL_CMD_GPU_GET_GID_INFO`, preceded by `HOST-ABI REFUSED control 0x2080014a ... host driver 595.91.07 is outside the interval the host encoders were written for`). The gate therefore passes an explicit per-VM `gpu-uuid=GPU-<VM_UUID>`. `win_vm.sh run` on a stock host without it refuses to start; that is a product limitation recorded here, not fixed here.
3. **The flag set is the sample list**, `scripts/bench/windows/kf3_flags.sample.env` (19 `KF3_*=1`), whose header says it is not confirmed as the production list; `KF3_RPC_TRACE=1` is on (the `run` default). This gate does not prove the flag-free default route.
4. **One die, one box, n=1 verdict.** g2 and g3 also reached Code 0 (g2 and g3 logs), and g3 and g4 passed the same graded probes inside the gate, so two gate runs on fresh overlays of one installed image agree (`g3_windows.log`, `g4_windows.log`; g3 was then failed by a gate bug, below). No other die or box was tried, and no native Windows reference was run on this box, so the BAR1 exception's native arm is untested here (host BAR1 was 256 MiB and kf3 realized with `bar1-size=134217728`).
5. `merge_check.sh` itself was not run with `KF_MERGE_WINDOWS=1` on this lease: the lease was a Windows lane box (no Linux fast guest or suites). The wiring is covered by `scripts/ci/test_vast_windows_gate.py`; the gate was run directly (`bash scripts/bench/box/windows_gate.sh g4`), which is the call the stage makes.

## The negative attempts (kept, not hidden)

| attempt | rev | verdict line | cause | fix |
|---|---|---|---|---|
| g1 | `ed37318e` | `WINDOWS_GATE_UNATTRIBUTED class=BAR1_EXCEPTION stage=boot1` after 27 s | kf3 refused to realize (`gpu-uuid=auto`, above). The BAR1 class was a gate bug: the heuristic matched the device's own argument list (`bar1-size=`) | explicit `gpu-uuid=`; BAR1 only from the reason text after `realize refused` (`windows_gate.sh`, commit `d18032f3`) |
| g2 | `d18032f3` | `WINDOWS_GATE_FAIL class=KAYFABE stage=smi_probes` after 920 s | gate bug, not kayfabe: `scp dir/. host:` is refused by Windows OpenSSH (`scp: error: unexpected filename: .`, reproduced by hand), and the harness failure was filed as a guest failure. Everything before it passed: Code 0, `nvidia-smi` | one scp per file; a failed scp is now OPERATOR, never KAYFABE (`13926b0f`) |
| g3 | `13926b0f` | `WINDOWS_GATE_FAIL class=KAYFABE stage=gsp_signals` after 1558 s | gate bug, not kayfabe: all guest checks and probes passed, then the final signature check counted the served trace line `rpc-trace fn=76 RmControl cmd=0x2080121f ... result=0x0` as the no-flags refusal | the signature is a `REFUSED` line naming `0x2080121f` (`a5071c24`) |
| g4 | `a5071c24` | `WINDOWS_GATE_PASS` after 1620 s | | |

g2's `FAIL class=KAYFABE` and g3's were misclassifications by the harness, found by reading the logs; they are the reason the harness now files a failed harness step as OPERATOR and never as KAYFABE. Three of the four attempts were the gate's own bugs; the guest never failed a check it was graded on.

## Files

`g1_windows.log` .. `g4_windows.log` (the gate logs, each ending in a verdict and `WINDOWS_GATE_EXIT`), `lane/` (provisioning and install logs), `g4/` (the evidence directory of the passing run: curated, text only, kf3 logs gzipped; GPU UUIDs (the box's board UUID and the per-VM guest UUID) are scrubbed to the repository placeholder, so the hashes in `g4/MANIFEST.sha256` of those files do not match; the manifest lists the full set the gate packed, including files not committed here: build output, host `nvidia-smi -q` dumps, serial logs, the TPM endorsement-key history, two PNG screenshots, `dxdiag`, the kernel PnP event dump). Not here, on purpose: the VM's `secrets/`, `tpm/`, `firmware/`, the Secure Boot db key, any Windows, NVIDIA or virtio binary, the probe executables (their sha256 are in `g4/probes.sha256`), the screenshots of the evaluation Windows (the owner has not answered whether they may be committed), and the box address.
