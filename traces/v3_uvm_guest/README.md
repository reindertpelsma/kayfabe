# v3-uvm-guest — the guest fault plane on the b3 host proof

Evidence for `docs/design/V3_UVM_GUEST_FAULT_PLANE.md`. Box `53564695` (alias `vuvm`): a rented
Vast **KVM-template VM** (nested), RTX 3060 / GA106 at PCI `0000:00:07`, host **open** NVIDIA kernel
module **580.159.04**, CUDA 12.6, the b3 patched `nvidia-uvm.ko` (`/root/efs/patched`, sha256 prefix
`b4bdee93fdfc151b`) loaded with `uvm_efs_enable=1 uvm_efs_timeout_ms=30000`. Guest: the fat bench
guest (stock 580.159.04 open modules). Workload: `traces/v3_uvm_research/um_probe.cu`, built on the
host with `nvcc -O2 -arch=sm_86 -cudart static` (md5 `1c21bdadd3fd52f7da7adc01d6275163`), one
process per mode; every mode checks every value it computed (`CHECK <mode> ok bad=0` is the only
pass). Driver: `scripts/bench/uvm_guest_lane.sh` + `scripts/bench/uvm_guest_hook.sh`, run with
`KF3_UVM_EFS=1` (plus `KF3_FAULTLOG=1 KF3_MAPLOG=1` for the diagnostic runs). The kf3 binary of each
run is `kf3-bins/<rev8>/` built from the named revision on the box (`run_<tag>_rev.txt`).

⚠ Nested box: memory-plane results should also be run on a non-nested host (§10 Q4); none was
reachable from this workspace on 2026-09-30.

## Runs, in order

| tag | revision | result | what it established |
|---|---|---|---|
| uvmg1 | `a4beda33` | every mode `cudaMalloc -> 401` | every EFS mirror refused: `UVM_REGISTER_GPU_VASPACE -> 0x5d NV_ERR_PAGE_TABLE_NOT_AVAIL` (the va_space had no `mm`; the ladder's W392C had learned it) — the refusal was hidden by the VA thread's log cap (fixed `49ea6f6d`, measured `uvmg2`) |
| uvmg3 | `89b1ac24` | `malloc`/`prefetch`/`advise` **ok bad=0**; `gpufirst`/`cpuinit` **719** | with `UVM_MM_INITIALIZE`: EFS-mode twins (host-UVM-owned space, external-mapping executor, `UVM_REGISTER_CHANNEL` before schedule) run CUDA correctly; faults are delivered (70) and replayed, then the context dies (`lane_uvmg3.log`, `run_uvmg3_hostdmesg.log`: Xid 109 CTX SWITCH TIMEOUT) |
| uvmg4 | `d178a737` | `gpufirst` **719** | ★ the diagnosis (`run_uvmg4_qemu.log.gz`, `KF3_FAULTLOG`+`KF3_MAPLOG`): 14 faults delivered at `t=5419.8899`; the walk kernel for the guest's servicing invalidate submitted at `5419.891` completed at `5424.1997` — **4.3 s**, only after the host ctxsw watchdog RC'd the twins (`5424.192`). ⇒ a GR context stalled on a parked replayable fault holds the GR engine; kf3's walker cannot run (design §3.8a) |
| uvmg5 | `b875595a` | `gpufirst` 719 | the CPU point-walk experiment fired but its views never armed (read-only access) and the replay `MEM_OP`'s dummy invalidate was still walked |
| uvmg6 | `aeaebe33` | ★ **`gpufirst` ok bad=0**; `cpuinit` 719; `malloc` ok | the first guest managed-memory result with correct data: 679 faults diverted → delivered → serviced by the guest → replayed (79 host replays), zero cancels. `cpuinit` hit a split/park race (fixed next) |
| uvmg7 | `c849f68d` | ★★ **all modes ok bad=0** — `gpufirst cpuinit prefetch advise malloc gpufirst cpuinit` | 3 499 records delivered, 265 host replays, 0 cancels, 0 host errors, **zero Xid** (`run_uvmg7_hostdmesg.log`: only the EFS shutdown accounting lines, `diverted=replayed` for every space) |

`run_uvmg6_qemu.filtered.log.gz` / `run_uvmg7_qemu.filtered.log.gz` are the device logs with the
per-page lines (`maplog … MAP va=`, `DOORBELL`, `faultlog DELIVER`, `faultlog EFSMAP`) removed for
size; the fault-plane timeline (`SPLIT-FAST`, `SPLIT-EARLY`, `POINT-MAP`, `GUEST-OP`, `RESOLVE`,
`EFSQ`) and every status/heartbeat line are kept. `run_uvmg4_qemu.log.gz` is unfiltered.

## What M2 means here, and what it does not

- ★ A guest `cudaMallocManaged` buffer first touched by the GPU (`gpufirst`), and one initialised by
  the CPU then used by the GPU (`cpuinit`), complete with **correct data** in a kf3 guest, with the
  guest's stock nvidia-uvm servicing real host replayable faults delivered by kf3 (`uvmg7`).
- ⊘ It needs the §3.8a experiment (a CPU point walk of parked pages beside the GPU walker) — a
  constraint change (owner ruling A.1/A.11) for the owner to rule on before anything lands.
- ⊘ Not yet run: the four apps (M3), the merge bar (M4), the Bug 1624521 negative control (E-S1),
  a non-nested host.
