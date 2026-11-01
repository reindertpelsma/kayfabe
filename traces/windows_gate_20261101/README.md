# Windows gate: the first real lease, 2026-11-01

STATUS: LIVE, 2026-11-01. Evidence of the first hardware runs of `scripts/bench/box/windows_gate.sh` (design:
`docs/design/V3_WINDOWS_GATE.md`). One Vast lease, four gate attempts on one installed Windows image, the last one a
`WINDOWS_GATE_PASS`. Labels: MEASURED = a line in a log in this directory, named with its file; INFERRED = reasoning.

## Per-die results (owner follow-up, 2026-11-01)

Gate semantics changed in the follow-up (`docs/design/V3_WINDOWS_GATE.md` section 2): only the guest passing exits zero; a host, harness or rental fault is `WINDOWS_GATE_RETRY` (non-zero). The verdicts quoted from the first lease below use the old `UNATTRIBUTED` word for what is now RETRY.

| die | box | gate revision | result | evidence |
|---|---|---|---|---|
| AD102 (RTX 4090) | lease 58592558, Pennsylvania US | g4 `a5071c24` | `WINDOWS_GATE_PASS` (n=1; g3 `13926b0f` had the same guest results and was failed by a gate bug) | `g4_windows.log`, `g4/` |
| TU106 (RTX 2060 SUPER, Turing) | lease 58656298, Quebec CA | t1 `8597d90e`, t2 `0860f249` | `WINDOWS_GATE_FAIL` twice (hostref green/green both times), plus a third negative boot (post-hoc, see below) | `tu106/` |
| GB20x (Blackwell) | not leased | | NOT SUPPORTED on Windows at `next` today: see the end of this file | |

### TU106 (MEASURED 2026-11-01, lease 58656298, `tu106/t1_windows.log`, `tu106/t2_windows.log`, `tu106/t2_posthoc/`)

- Box: RTX 2060 SUPER (8192 MiB, TU106) behind a 21 vCPU / 24 GB guest, 0.107 USD/h, created 2026-11-01T12:19Z, destroyed 15:11Z (about 2 h 52 min, 0.34 USD from the account credit 40.73 to 40.39). Host driver 595.91.07 open. The gate sized `fb-mb` from the VRAM itself (`WINVM_KF3_FB_MB=6144`; `tu106/t1_windows.log`, `GATE_NOTE host VRAM 8192 MiB`).
- Lane: `tu106/lane/winlane.log` `WINLANE_TIMING total=3392s ... install_windows=2446s` (the install took 41 min here against 31 min on the 4090 box).
- Both gate runs got as far as the AD102 run did: ssh on the kf3 guest, `check.ps1 --kf3` `WIN_CHECK_DONE fail=0`, GSP keys read back, signed-in desktop, the one-time enable, Code 0 at driver 32.0.15.8088, `nvidia-smi` with the GPU (`t1/nvidia_smi_L_guest.txt`), host reference green before and after. Turing therefore reaches Code 0 with the same keys and `guest-driver=580.65.06`.
- **t1 (`8597d90e`): `WINDOWS_GATE_FAIL diag=KAYFABE stage=smi_probes why="graded probes failed: cup8"`.** `cup8` `FAIL cuInit(0) -> unknown error (999)` (`t1/probe_cup8.txt`), `d3d11` OK, `cup3` OK, `d3d12` `KFDX RESULT FAIL D3D12 compute objects created` after 103 s. The kf3 log shows 16 `Running -> Suspending` RM unloads in the boot (`t1/gsp_signals.txt`, `running_to_suspending=16`). Host Xid 0.
- **t2 (`0860f249`, same installed image, a fresh overlay and a fresh QEMU, not a fresh box): `WINDOWS_GATE_FAIL diag=KAYFABE stage=post_idle why="graded probes failed AFTER the idle window: cup8"`.** The first probe round passed (`cup8` rc=0 in 56 s, `d3d11` rc=0, `cup3`, `d3d12` rc=0 in 106 s; the round took 646 s against 85 s on the 4090), one `ssh miss` came at 180 s of the idle, and after the 300 s idle `cup8` failed at `cuCtxCreate` with 999 (`t2/probe_cup8_post.txt`) and the guest then stopped answering (`t2/state_at_failure_*.txt` are empty: the commands timed out). The kf3 log shows 42 RM unloads (`t2/gsp_signals.txt`, `running_to_suspending=42`). Host Xid 0, host reference green after.
- **Post-hoc read of the t2 overlay** (a manual boot after the gate, tag `posthoc2`, with the new `gate_state.ps1 -Mode wer`; `t2_posthoc/`): since the t2 enable at 14:11:43Z the guest logged `nvlddmkm 153 = 147`, Kernel-Power 41 = 2, EventLog 6008 = 1, BugCheck 1001 = 2, LiveKernelEvent other than 193 = 1006, LiveKernelEvent 193 = 43 (`tdr_counts_posthoc.txt`). The post-hoc boot itself panicked about 3 minutes after it started (`posthoc2_run.log`, `GUEST_PANICKED`). The WER reports still on disk are 49, all `LiveKernelEvent` code 117 (`wer_summary.txt`; full text `wer_reports.txt.gz`): the first has event time 14:14:09Z, 2 min 26 s after the enable, and the first `nvlddmkm` 153 came at 14:13:58Z, 11 s before it.
- So on TU106 this gate fails on a reset storm, as `traces/tur1_20261027/README.md` (leases 57882297, 57908255) found on the same die, with two different outcomes between boots also seen there (t1 failed at the first round, t2 at the second). This is n=2 gate runs plus the post-hoc boot (three negative boots on one box); a third negative boot on a FRESH box was not run. The rule "max 3 negative boots per die" was applied to boots on this box, and the cause of the storm is not found here.
- Not done: a fresh-box rerun for TU106, the native Windows reference on this machine, and the LiveKernelEvent 193 timing on TU106 (the 43 events have counts but no kept report or dump: `dump_files=0`, only code-117 reports survive).

## The AD102 lease (the first one)

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

1. **WATCHDOG live dumps, LiveKernelEvent 193 (open defect; the gate reports it and does not gate on it unless `KF_WINGATE_STRICT_LKE=1`).** Per die:
   - **AD102** (MEASURED 2026-11-01, lease 58592558): g3 and g4 each show `livekernelevent193=2` with all other reset counters zero (`g3_windows.log`, `g4/tdr_counts.txt`) and one 1.4 GB `WATCHDOG-*.dmp` (`g4/guest_collect/dumps.txt`: 01:58:06Z; g3: `WATCHDOG-20261101-0129.dmp` 01:29:57Z). Timeline in the guest/host clocks: g4 enable 01:57:00Z, dump 01:58:06Z (+66 s), first probe round started 01:58:15Z (host clock, `g4_windows.log` `smi_probes START`; +9 s after the dump); g3 enable 01:28:44Z, dump 01:29:57Z (+73 s), probes 01:30:08Z (+11 s). So in both runs the dump PRECEDED the first probe, and the probes (before and after a 300 s idle) passed and no reset followed. The report text (MEASURED by a manual query during the g2 debugging, not saved as a file, the lease is gone): `EventName LiveKernelEvent`, `P1 193, P2 815, P3 2, P4 ffff8185c66c5080, P5 0, P6 10_0_26100`; two such reports 0.14 s apart at 01:05:24Z in g2 (dump 01:04:59Z) and another at 01:11:56Z in the debug boot; the System-log filter of that query found no `nvlddmkm`, `Display` or `dxgkrnl` event around them. The new `gate_state.ps1 -Mode wer` (Report.wer parameters, dump times, events within +-120 s) was written after this lease ended, so it has not run on the AD102.
   - **TU106** (MEASURED 2026-11-01, lease 58656298, `tu106/t2_posthoc/`): 43 LiveKernelEvent-193 events since the t2 enable among 1006 other LiveKernelEvents and 147 `nvlddmkm` 153; no 193 report or dump survives on disk (`dump_files=0`, 49 kept reports are all code 117), so the order of the 193 events against the first reset (`nvlddmkm` 153 at 14:13:58Z, first code-117 report 14:14:09Z) is NOT determined. Whether a 193 precedes the TU106 resets is open.
   - **Blackwell**: no run (family not supported on Windows at `next`, below).
   - **Falsifier kept**: any run where a 193 dump precedes a reset, a TDR or a probe failure. Not triggered on the AD102 (the dump preceded only passing probes); not decidable on TU106.
   - The dump itself is never downloaded (1.4 GB); the gate copies only the Report.wer text.
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

## Blackwell (GB20x): not leased, and why (CODE READ 2026-11-01 at `next` `9297ffdd9` and `docs/design/V3_FAMILY_COVERAGE.md`)

The owner asked for one Blackwell consumer die only if kf3 supports the family on Windows today. At `next` it does not:

- The Windows results on record for Blackwell were all taken on branches that are not in `next`: GB205 Code 0 and a 601 s idle hold at `007ba1eb0` (`claude/blackwell-fixes-20261024`, `git merge-base --is-ancestor 007ba1eb0 HEAD` is false), GB202 Code 0 then `0x141`/`0x113` (BW6) and GB206 Code 43 / Xid at `cb80cbd29` (also not in `next`).
- The relay-route fix that those runs needed is absent from `next`: commit `b85d02bae` makes a family whose engine writes no USERD word take the USERD-relay route (`kf_chip::Family::engine_writes_userd_gp_get`); `grep -rn engine_writes_userd_gp_get crates` at `next` finds only a test and a comment, no implementation. So a Blackwell Windows USER channel is admitted to the private ring that cannot prove a marker (`USER_RING_FAMILY_WRITES_NO_USERD` in that commit's message).
- The measured Blackwell display pages for the Windows display path (`619c812b0`, `48b027267`, GB202/GB206 PRECOMP rows) and the second-life fix (`38fd948fd`) are on the same unmerged branch; `V3_FAMILY_COVERAGE.md` lists GB203/GB205 Disp-W as PARTIAL with `0x193/0x1A8/0x1B8` dumps.
- The driver is not the blocker: the pinned Windows 580.88 supports RTX 50xx and the host 595.91.07 open driver supports Blackwell. What is missing is the code of `claude/blackwell-fixes-20261024` (503 commits not in `next`), to be merged and validated first.
