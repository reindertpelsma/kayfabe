# V3 multi-GPU audit — is it still possible, and is it required?

**STATUS: LIVE, 2026-09-26.** Audit of `master` at `79848341`. Hardware runs used the bench
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

**Mandate violations today:** §w724f's *"CUDA context bound to a device"* (blocker 1), and its
per-card BAR1 budget, which is not implemented. `THE_V3_PLAN.md:385`'s *"no single-GPU
assumption"* holds for `kf-chip` and the object graph. It is broken only at the two host-side
selection points above.
