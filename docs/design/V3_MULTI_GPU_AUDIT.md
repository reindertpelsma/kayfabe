# V3 multi-GPU audit — is it still possible, and is it required?

> **STATUS (updated 2026-09-26, branch `v3-mgpu`): LIVE — blockers 1 and 2 FIXED, harness BUILT,
> per-card BAR1 budget check BUILT (in-process scope).** See §6 for the fixes and §7 for what was
> measured on a multi-GPU box. §2's blocker text below is kept as the audit found it; each blocker
> now carries a ✔ line naming its fix.

**Original status: LIVE, 2026-09-26.** Audit of `master` at `79848341`. Hardware runs used the bench
binary `kf3-bins/4e9aae9b`. Between `4e9aae9b` and `79848341` only docs and `kf-crec` tests
changed, so the binary matches the audited code. This is an audit only: no code was changed.

**Answer.** Multi-GPU is still possible, and the design requires it. The code keeps almost all
state per device, as the design asked. The same host GPU exposed twice works today, measured
below. Two distinct host GPUs are blocked by two small bugs. Both select the host GPU by
**ordinal or minor**, and neither selects it by identity. Peer access and a shared BAR1 budget
are still open design questions, as the design already records.

---

## 1. What the design says

| scenario | posture | source |
|---|---|---|
| **one kf3 device per host GPU; several in one VM** | ★ **MANDATED (owner 2026-09-26)** | `V3_GUEST_DOORBELL_MODULE.md:96-98`: *"Multiple GPUs — required on both sides. kayfabe: one kf3 device per host GPU, each with its own host RM session, host doorbell page, doorbell table and table BAR."* |
| per-GPU walker / store / CUDA ctx / format descriptor / BAR1 budget | ★ **MANDATED (owner 2026-09-14)** | `THE_CONSTRAINTS.md:2039-2069` §w724f: *"multiple gpu still remains an axis, so multiple scratchpads if needed each with a ptx for tables in that gpu"*; the table lists GPGA space, reserved object, CUDA context (*"bound to a device"*), walk launch, `KfSetup`, and BAR1 budget as per-GPU, and *"none of it is optional"* |
| mixed generations in one process (e.g. Turing + Blackwell) | ★ implied by the above | `THE_CONSTRAINTS.md:2063-2069`: *"VER2 and VER3 live simultaneously, in one kayfabe process"* |
| no single-GPU assumptions in `kf-chip` / object graph | ★ **MANDATED** even while the feature is deferred | `THE_V3_PLAN.md:385-386`: *"No multi-GPU. ⚠ But `kf-chip` and the object graph must not *assume* one GPU — … key by GPU from the start"* |
| building/testing multi-GPU as a v3 phase | ⊘ **DEFERRED** (2026-09-20 plan); the 09-26 ruling moves it to *"its own test before the module"* | `THE_V3_PLAN.md:385`; `V3_GUEST_DOORBELL_MODULE.md:102-103` (*"(distinct host GPUs, and later the same GPU twice)"*) |
| the same host GPU in several VMs | ◐ **assumed, not specified**. The only mentions: a *"shared host GPU"* error policy, and cross-tenant BAR1 reuse | `THE_V3_PLAN.md:305-309` (P5.5); `THE_CONSTRAINTS.md:2206`, `:2284-2291`, `:2465-2470` |
| **peer access between guest GPUs** | ⚠ **OPEN**. *"decide it explicitly, even if the decision is a named refusal"* | `THE_CONSTRAINTS.md:2071-2079` |
| shared-pool BAR1 headroom (second GPU / second VM) | ⚠ **OPEN** | `THE_CONSTRAINTS.md:2166-2168` |
| hot-added second GPU | ⊘ treat CUDA re-init with a guest live as a **refusal** | `THE_CONSTRAINTS.md:2027-2032` |
| `deviceId` routing | stated as *"decoded for multi-GPU routing"* | `THE_SURFACE_v3.md:157`. See §3 note: on the GSP wire the value is always 0 |
| chip tables derivable per die | ✔ ruled R11 | `THE_ARCHITECTURE_v3.md:1228` |

## 2. Code audit — what is per device

`kf3_realize` (`crates/kf-qemu/src/ffi_unsafe.rs:79-128`) builds one `Device` for each QOM
instance and returns it as the handle. Each device gets:

- its own drainer, two workers, and a VA thread;
- its own `HostRm`, meaning its own root client, `nvidiactl` fd and usermode (doorbell) window
  (`device.rs:178`, `kf-host/src/lib.rs:319-470`);
- its own `Family` and MMU format descriptor (`device.rs:199-204`);
- its own `WalkKernel`, meaning its own CUDA context, made current on its own VA thread
  (`device.rs:756`);
- its own store (`device.rs:206`) and GSP FSM;
- its own served RM chain and `RmGraph` (`device.rs:310`);
- its own channel plane and token table (`device.rs:286`), CPU interrupt tree, and MSI-X
  eventfds.

`kf3.c` has **no mutable file-scope state**. BARs, the RAM listener and irqfd routes all live in
`Kf3State`, and memory-region names are scoped to their owner. The ⊘ `Box::leak`s are per
realize, which means a device cannot be unplugged. That applies equally to one device.

The process-global statics are **diagnostics only**:

- `kf-util` trap and lock witnesses;
- `kf-qemu/src/prof.rs`;
- the log cap `kf-rm/src/rmrpc/policy.rs:449`.

With two devices they aggregate, or share a cap. That is cosmetic, not a correctness problem.
There are no lock files, fixed `/tmp` paths or singleton fds in the crates.

### Blockers for **distinct** host GPUs (small fixes)

1. ★★ **The CUDA walker always opens ordinal 0.** See `crates/kf-cuda/src/walk.rs:657`
   (`cu.device_get(0)`) and `:1547` (`import_and_map(0, …)`). A kf3 device with `gpu-minor=1`
   would create its walker context on some other GPU, then import the store it exported from
   GPU 1 into that context. CUDA orders ordinals *fastest-first*, not by PCI address or minor,
   and `CUDA_VISIBLE_DEVICES` can reorder them.
   - This **violates §w724f** (*"the CUDA context — bound to a device"*).
   - Fix: pass the host BDF from `hostfacts::sysfs_dir_for_minor` into `WalkKernel::bring_up`.
     Choose the ordinal by `cuDeviceGetPCIBusId` (or UUID), then use that ordinal for the
     import.
   - ✔ **FIXED (`v3-mgpu`, §6.1).** `WalkKernel::bring_up_on(.., WalkDevice::PciBusId(bdf))` via
     `cuDeviceGetByPCIBusId`; the import's `cuMemSetAccess` names the same `CUdevice`.
2. ★★ **The host `NV01_DEVICE_0` is allocated with `deviceId = minor`.** See
   `crates/kf-host/src/lib.rs:405`. RM's `deviceId` is the *device instance*, which RM assigns
   when the GPU attaches. On Linux without persistence mode, attach happens at first open, and
   RM takes the lowest free instance. So the minor and the instance diverge whenever GPU 0 is
   not attached.
   - **Measured, in the guest, below (§4 run 1):** `/dev/nvidia1` got instance 0 after GPU 0
     detached. `deviceId=1` was then refused with `Other(34)` (`NV_ERR_INVALID_CLASS`), and
     `gpumgrGetPrimaryForDevice: deviceInstance 0x1 does not exist!` was logged.
   - The raw client has the same defect: `kayfabe-isolate-host/src/rm.rs:3317` and
     `kayfabe-rm-ladder/src/main.rs:13110`.
   - Fix: resolve the instance with `NV0000_CTRL_CMD_GPU_GET_ID_INFO_V2`, reached through
     `GET_ATTACHED_IDS` or `GET_PROBED_IDS` and matched on the PCI BDF. Do this after
     `REGISTER_FD`, which is the step that attaches the GPU.
   - ✔ **FIXED (`v3-mgpu`, §6.2)** — by a shorter route than the one proposed: the frontend's own
     `NV_ESC_CARD_INFO` already maps **minor → RM `gpuId` + PCI address**, so no BDF matching over
     `GET_ATTACHED_IDS` is needed; `GET_ID_INFO_V2(gpuId)` then gives `deviceInstance` and
     `subDeviceInstance`.
3. ◐ **Family-axis constants.** `kf_trap::memmap::VF_USERMODE_PAGE` (`kf-trap/src/memmap.rs:124`)
   is one constant, used for every family at `device.rs:413`, `:534`, `:561` and `:565`, and at
   `mem.rs:711`. The doorbell module doc says the page differs on Hopper and Blackwell. If it
   does, a mixed-family VM breaks — but so does a single non-GA10x GPU. This belongs to the
   family axis, not to multi-GPU. Everything else about family is already per device.

### Needs design work (open in the design already)

- **Shared host BAR1.** Every kf3 device over one host GPU takes views from one host BAR1
  aperture: 256 MiB on the bench 3060, which has no ReBAR. There is **no accounting and no
  startup refusal** in v3, and `THE_CONSTRAINTS.md:2166` leaves `our_headroom` undecided.
  2×128 MiB guest BAR1 booted fine (§4), because views are taken lazily. Exhaustion would
  therefore show up at run time, not at realize.
- **Peer apertures.** A peer PTE is refused by name in the CE translator
  (`kf-chan/src/translated.rs:449`, `Refusal::PeerOperand`) and in the ledger
  (`kf-mem/src/ledger.rs:27`). P2P capability controls fall to the unserviced ledger as
  `NOT_SUPPORTED` (`THE_SURFACE_v3.md:236`). That is a *de facto* named refusal. It meets the
  letter of §w724f. It has not been decided explicitly or tested against a guest UVM with two
  GPUs.

### Harness (single-device by construction)

- `scripts/fastguest/run_fast_guest.sh:250` and `scripts/bench/boot_nvkvm.sh:29` emit exactly
  one `-device kf3-gpu,…,id=kf0`. `KF3_DEV_EXTRA` can only append properties to that one device.
- `scripts/fastguest/build_fast_guest.sh:372` creates only `/dev/nvidia0`, and `:410` runs only
  `--gpu 0`.
- `run_fast_guest.sh:63` (global flock) and `:74` (`pkill -9 -x qemu-system-x86`, which kills
  **every** QEMU) rule out several VMs on one host by design.

## 3. Note on `deviceId` inside the guest's GSP traffic

ogkm forwards a Device alloc to GSP through `NV_RM_RPC_ALLOC_SHARE_DEVICE`, which builds
`NV0080_ALLOC_PARAMETERS device_alloc_params = {0}` and never sets `deviceId`
(`research_clones/ogkm-580.159.04/src/nvidia/inc/kernel/vgpu/rpc.h:96-106`). So every guest GPU's
GSP sees `deviceId = 0`.

With one kf3 device (one GSP) per guest GPU, the device that received the RPC identifies the
GPU. The `RmGraph` default entitlement `{0}` (`kf-rm/src/rmgraph.rs:1049`) is correct per device.
`THE_SURFACE_v3.md:157`'s *"decoded for multi-GPU routing"* has nothing to route on this path.
Confirmed by §4: both guest GPUs reached `Running` with no `InvalidDeviceInstance` refusal.

## 4. Hardware — `vh2` (RTX 3060, host 580.159.04), one host GPU

Scratch dir `/workspace/bench/mgpu-audit/` on the bench:

- the fast-guest initrd was patched so that `/init` makes `/dev/nvidia1`, holds both nodes open,
  and runs `rmladder --ce-client` on `--gpu 0`, then `--gpu 1`, then both concurrently;
- `-device kf3-gpu,gpu-minor=0,fb-mb=4096,…` was passed twice.

| run | result |
|---|---|
| 2 devices, same host GPU, one VM | both realize (store 4096 MiB each, walker imported each). Guest sees `00:03.0` and `00:04.0` as `10de:2504`, minors 0 and 1. `RmInitAdapter` succeeds on both, and **both GSP FSMs reach `Running`**. The raw CE client is **ALL ARMS MET on GPU 0 and on GPU 1**, sequentially and concurrently. 4 doorbells `route=passthrough forwarded=1` |
| run 1 of the above (no node held open) | `--gpu 1` → `R5 NV01_DEVICE_0: Other(34)` + `deviceInstance 0x1 does not exist!` — blocker 2, measured (client side) |
| 2 devices × 8192 MiB on the 12 GiB card | device 2 **refused at realize, by name**: `store of 8192 MiB refused: NoMemory`. Loud, as intended |
| 2 QEMU processes, 1 device each, same GPU, overlapping in time | both boot, both `Running`, and both CE clients are ALL ARMS MET (2 forwarded doorbells each) |

The only failure seen is `R33 arm 6 CALIBRATION` FAIL. It is **identical on single-device
baselines** (`fast_mc4_ce-client`, `fast_base_f8_ce-client`), so it is not a multi-GPU effect.
⊘ Not covered: distinct host GPUs (the bench has one); CUDA or UVM in a two-GPU guest; the fat
guest. Also seen: a guest reboot inside one QEMU process fails `RmInitAdapter` (`0x62:0x40`).
That is the known P5 teardown gap, and it is unrelated to multi-GPU.

## 5. Verdict per scenario

| scenario | verdict |
|---|---|
| several kf3 devices over **one** host GPU, one VM | ✔ **works as-is** (measured, raw CE client) — up to the fb and BAR1 budget |
| several **VMs** on one host GPU | ✔ **works as-is** (measured, 2 VMs). The harness forbids it (`pkill`, flock); BAR1 accounting is design work |
| several kf3 devices over **distinct** host GPUs, one VM (the mandated case) | ✘ **small fixes**: blockers 1 and 2 (`kf-cuda/src/walk.rs:657,1547`; `kf-host/src/lib.rs:405`), plus a harness that can emit N devices and N nodes. Untestable on a one-GPU bench |
| two GPU **families** in one VM | ◐ threaded per device. The `VF_USERMODE_PAGE` constant is a family-axis question to settle anyway |
| multi-GPU **inside** the guest (peer / P2P) | ⚠ **needs design work**. Refused by name today, never decided or tested |
| hot-add | ⊘ out of scope per `THE_CONSTRAINTS.md:2027-2032` |

> ✔ **Superseded 2026-09-26 by §6/§7:** the distinct-host-GPU row is now ✔ **measured working**
> (two and three-apart minors, serial and concurrent, `/dev/nvidia0` held and not held), and the
> per-card BAR1 budget is checked at realize (in-process scope). The rows below are the audit's
> verdicts as found.

**Mandate violations today:** §w724f's *"CUDA context bound to a device"* (blocker 1), and its
per-card BAR1 budget, which is not implemented. `THE_V3_PLAN.md:385`'s *"no single-GPU
assumption"* holds for `kf-chip` and the object graph. It is broken only at the two host-side
selection points above.

---

## 6. The fixes (branch `v3-mgpu`, `74fced87`)

### 6.1 Blocker 1 — the walker's CUDA device is chosen by PCI address

- `kf-cuda`: `Cuda::device_by_pci_bus_id` (`cuDeviceGetByPCIBusId`, resolved with `sym!`, so a
  driver without it refuses the binding by name). `WalkKernel::bring_up_on(cfg, fmt, WalkDevice)`
  with `WalkDevice::PciBusId(bdf)`; `bring_up` stays as `WalkDevice::FirstOrdinal` for callers
  that have only one GPU by construction. The kernel keeps its `CUdevice`, and `import_store`'s
  `cuMemSetAccess` names it (it was a literal `0` too — a second, quieter instance of blocker 1).
- `kf-qemu` realize passes `HostRm::card().bdf()`. It refuses by name if procfs's
  `Device Minor` → BDF disagrees with `CARD_INFO`'s, and prints one identity line per device:
  `kf3: host GPU minor=… bdf=… gpuId=… rm_device_instance=… cuda_device=… store=…`.
- `kf-harness` gates: `KF_GATE_GPU=<minor>` (default 0) selects the host GPU; every gate that
  opens `HostRm` brings its walker up on `rm.card().bdf()`; gate 9 (no RM session) uses the
  procfs BDF of that minor. ⊘ Before this, on a multi-GPU box a gate could run its RM half and
  its CUDA half on different GPUs.
- ⊘ Not changed: the legacy `kayfabe-cuda` crate (`walk.rs:444`, `:940`) and
  `kayfabe-isolate-host/src/cudastore.rs:262` still use ordinal 0. They are the v2
  `cuda-scratchpad` isolate, which kf3 does not link.

### 6.2 Blocker 2 — the RM device instance is resolved, never assumed

- `kf-abi` / `kayfabe-abi` (`bringup.rs`): `NV_ESC_CARD_INFO` (200) + `CardInfo` (72-byte
  `nv_ioctl_card_info_t`, layout pinned by a `#[repr(C)]` mirror test against `nv-ioctl.h`), and
  `NV0000_CTRL_CMD_GPU_GET_ID_INFO_V2` (0x205) + `GpuIdInfoV2`.
- The resolution, after the per-GPU open + `REGISTER_FD` (which attaches the GPU) and the root
  client: `CARD_INFO` on `nvidiactl` → the entry whose `minor_number` is ours → its `gpu_id` and
  PCI address; `GET_ID_INFO_V2(gpuId)` on the root client → `deviceInstance`,
  `subDeviceInstance`. Those go into `NV0080_ALLOC_PARAMETERS.deviceId` and
  `NV2080_ALLOC_PARAMETERS.subDeviceId`. A minor the frontend does not list is refused by name
  (`R4b CARD_INFO minor`).
- Sites: `kf-host::HostRm::open`; `kayfabe-isolate-host::rm::resolve_device_instance` (one
  function) used by `RmConnection::open`, `BirthConn::open` (route K's B tree, which had the same
  defect via `gpu_index`) and both `kayfabe-rm-ladder` arms (`K_BIT5_KP`, route K's S side).
- `kayfabe-rm-ladder`'s UVM `gpu_uuid(gpu_index)` picked the N-th **sorted** procfs entry; it now
  matches the entry's own `Device Minor:` line.

### 6.3 Harness

- `run_fast_guest.sh`: `KF3_GPUS=0,1` emits one `-device kf3-gpu,gpu-minor=<m>,…,id=kf<i>` per
  listed host minor (unset = the single-device lane, unchanged). Kernel-line contract:
  `KF_NGPU`, `KF_MGPU_MODE` (`serial`, `concurrent`, or both), `KF_HOLD0`.
- Guest `/init` (`build_fast_guest.sh`): makes `/dev/nvidia0..N-1`; runs the client per GPU,
  serially **highest minor first** with `/dev/nvidia0` closed unless `KF_HOLD0=1` (the guest
  renumbering case), then concurrently; each run's rc goes to a file and `client rc=` is 0 only
  if every run was 0 (a run that left no rc counts as a failure).
- QEMU is started with `-name kf-fastguest` and the lane kills only `pkill -9 -f
  '[k]f-fastguest'` — no longer every `qemu-system-x86` on the box. The GPU-drain wait takes
  the max over all GPUs.
- ⊘ Not done: the doorbell-ledger grader is not device-scoped. Two devices print rows with the
  same `tok=` values; the per-token stranded rule still holds per row, but a row does not say
  which device it came from. The fat-guest `boot_nvkvm.sh` is still single-device.

### 6.4 Per-host-card budget — refused by name at realize (`kf-qemu/src/cardbudget.rs`)

    Σ over this process's kf3 devices on one host card of
        (guest BAR1 + guest BAR2 + 1 MiB PRAMIN + 16 MiB OUR_HEADROOM)  ≤  host BAR1

Host BAR1 is read from sysfs `resource` line 1, never a literal. The store keeps RM as its
budget (the reservation refuses with `NoMemory`), and the refusal now names how much store this
process's other devices already hold on that card.
- ⚠ **Behaviour change, stated:** two default devices (128 MiB BAR1) on **one** 256 MiB card —
  which §4 measured as booting, because views are lazy — are now **refused at realize** (§7).
  Two 64 MiB-BAR1 devices fit (2 × 113 MiB). That is the design's own rule
  (`THE_CONSTRAINTS.md` §w727: *"refuse at startup, loudly, never silently clamp"*), applied
  across devices.
- ⚠ **Scope:** one QEMU process. Two VMs on one card share the host BAR1 pool, and neither sees
  the other's demand. Making that visible needs a host-wide ledger (a per-card lock file or a
  broker). That, and what `OUR_HEADROOM` must contain (`THE_CONSTRAINTS.md:2166`), stay design
  work.

## 7. Measured — box 52664396 (8× RTX 3060 GA106, host 580.159.04 open, persistence OFF), `74fced87`

Scratch dir `/workspace/bench/mgpu/` on the box. Every GPU job ran serially. The box was
destroyed after the runs. Three 2-GPU offers failed first (vast `GPU error, unable to start
instance` twice, and one stuck `waiting for previous instance to free memory`), so the run used
2 of the 8 GPUs on an 8-GPU box. Crate tests (local): `kf-abi`, `kayfabe-abi`, `kf-cuda`,
`kf-host`, `kf-qemu` (incl. 4 new `cardbudget` tests), `kf-harness`, `kf-util` all green.
⊘ `kayfabe-isolate-host --lib` has one failure, `rm::tests::no_collision_across_the_workspace`
(refusal integers named in both `rm.rs` and `kf-host/src/lib.rs`). It is pre-existing: the
colliding constants are on `origin/master` and this branch adds none.

| test | result |
|---|---|
| **falsifier**: pre-fix client (`origin/master` `79848341`), bare metal, `--ce-client --gpu 1`, nothing else open | ✘ `RM bring-up failed at R5 NV01_DEVICE_0: Other(34)` — blocker 2 reproduced on the **host** (no persistence ⇒ GPU 1 attaches alone as instance 0) |
| fixed client, bare metal, `--ce-client --gpu 1`, then `--gpu 0`, then `--gpu 5` | ✔ rc=0 ×3, `ALL ARMS MET`; census rows 4–5 are `CARD_INFO` (2304 B) and the `GET_ID_INFO_V2` control, both OK |
| v3 gates, `KF_GATE_GPU=0` | ✔ 9/9 (`V3_GATES_SUMMARY pass=9 fail=0`) |
| v3 gates, `KF_GATE_GPU=1` (same binaries) — the per-device CUDA walker + store-import smoke | ✔ 9/9 |
| thin guest, `KF3_GPUS=0,1`, `KF_HOLD0=0`, `--ce-client`: serial (gpu1 then gpu0), then concurrent | ✔ `FAST_VERDICT=PASS (38s)`; all 4 runs rc=0 and `ALL ARMS MET`, each reporting its own `GPU 0`/`GPU 1`; `8213 doorbell(s) forwarded across 5 guest token(s); 0 stranded`. Realize: `minor=0 bdf=0000:00:07.0 gpuId=0x7 rm_device_instance=0` and `minor=1 bdf=0000:00:09.0 gpuId=0x9 rm_device_instance=1` |
| same, `KF_HOLD0=1` | ✔ PASS (43s), all 4 runs rc=0 |
| thin guest, `KF3_GPUS=3,6` (host minors ≠ guest minors) | ✔ PASS (26s), all 4 runs rc=0. ★ Realize printed **`minor=3 … rm_device_instance=5`** and **`minor=6 … rm_device_instance=2`**: on the host, minor and instance differed for BOTH devices, and `deviceId = minor` would have named the wrong GPU or none |
| thin guest, `KF3_GPUS=0,0` (same card twice, 128 MiB BAR1 each) | ✔ refused at realize, by name: `host card 0000:00:07.0: BAR1 budget exceeded — this device needs 177 MiB … the 1 kf3 device(s) already on this card hold 177 MiB; 354 MiB > the host's 256 MiB BAR1` (the harness scores it CRASH: empty serial log, as intended) |
| **regression**: thin guest, ONE kf3 device (GPU 0), 30-arm `fast_suite.sh`, 120 s | ✔ **30/30** — `FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0` (23:51→00:13 UTC). The single-device lane did not regress |

⊘ Not measured: CUDA **inside** a two-GPU guest (the thin guest has no libcuda), guest UVM
across two GPUs, peer access, and the fat guest. The per-device CUDA evidence is the host side:
each kf3 device's walker context and store import on its own GPU (the identity lines above, and
gates 2–9 on GPU 1).
