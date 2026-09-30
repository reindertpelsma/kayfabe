# kayfabe

Kayfabe emulates an NVIDIA GPU inside QEMU. The guest sees an emulated PCI device plus a faked
GSP, and loads the **real, unmodified NVIDIA driver** against it (stock 580.159.04, open kernel
modules). Nothing in the guest is patched or shimmed. Kayfabe recovers what the guest driver is
asking for from its own protocol (GSP RPCs, page tables, doorbells) and has a real host GPU do the
work, from an **unprivileged** host process that talks to the host driver as an ordinary RM
client.

What that buys, and why the project exists:

- **Hostile-guest isolation.** The guest owns guest root, its driver and its userspace; none of
  them is trusted. Every host action is *authored* by kayfabe from a small set of unprivileged
  RM verbs, never forwarded from the guest. This is the value proposition
  ([`docs/PRODUCT_POSITIONING.md`](docs/PRODUCT_POSITIONING.md) §2).
- **The guest OS stops being a constraint.** You cannot ask Windows to load your kernel module,
  but you can let it load NVIDIA's. Windows guests are the end goal.
- **Guest and host driver versions are decoupled** by design.

It is written in Rust. The architecture is **v3** ([`ARCHITECTURE.md`](ARCHITECTURE.md),
[`docs/design/THE_ARCHITECTURE_v3.md`](docs/design/THE_ARCHITECTURE_v3.md)). The C research
prototype that first proved the idea is frozen under [`archive/nvkvm/`](archive/README.md).

## Status — 2026-09-30

`master` is the code of `afb552ea`, which passed the full hardware merge bar on an RTX 3060: all
**1,754** `kf-*` crate tests, the **nine GPU gates**, a kf3 build, and the 30-arm raw client both on
**bare metal (30/30)** and in the **thin guest (30/30)** ([evidence](traces/v3_mc23/README.md)).
GitHub CI is green, including the slow suite. For current decisions and the resume point, see
[status and handoff](docs/STATUS_AND_HANDOFF.md).

**Research stage, published so the approach can be read and argued with.** There is no install path
and no stable interface. Every result below was measured on rented vast.ai boxes that are themselves
KVM guests (so the guest under test is *nested*), host driver 580.159.04 (open) unless noted.

Works, measured:

- **Every GPU family from Turing to Blackwell runs the stock driver.** At the current code, TU116
  (GTX 1660 SUPER), AD104 (RTX 4000 Ada) and GB205 (RTX 5070) each pass the merge bar and the CUDA
  ladder in the guest ([Turing](traces/v3_turing_master/README.md),
  [Ada, Blackwell](traces/v3_families_master/README.md)); GA106 is the everyday bench. Earlier
  revisions also passed on GA104, GA102, AD106, GB203 and GB206, and on a floor-swept RTX 3060 Ti.
  Hopper, GA100 and GB10x are derived from the open driver's source only (no hardware was available);
  GA100 and GB10B are refused by name.
- **CUDA is correct:** `cup3` returns 43, `cup8` (2048² matmul) returns `BAD=0 MAXERR=0`, and every
  timed iteration of `cup8bench` verifies.
- **Real applications: 61 of 65 pass** in the guest (host: 65/65), plus 6/6 stream probes, and
  100 CUDA processes run in one boot. These include PyTorch (a CNN training step with the same digest
  as the host), Hugging Face generate, llama.cpp (tokens identical to the host), CuPy, hashcat,
  Blender CUDA+OptiX, Geekbench, clpeak, gpu_burn and the CUDA samples, including dynamic
  parallelism ([app matrix](docs/design/V3_APP_MATRIX.md), [CDP](docs/design/V3_CDP.md)).
- **Headless graphics: nvkvm-pv's headless test set plus 15 more items, 38/38**, 31 of them
  byte-identical to bare metal: Vulkan, EGL, and GLX through Xvfb + VirtualGL
  ([test set](docs/design/V3_GFX_TESTSET.md)). **NVENC and NVDEC output is byte-identical** to bare
  metal.
- **Display (opt-in, `display=on`):** the guest's own NVIDIA display driver drives an emulated
  monitor — pixel-exact 1920×1080 scanout at 60 Hz, Linux Mint's Cinnamon desktop on Wayland with
  `vkcube` in a window, weston, and Xorg with the NVIDIA X driver
  ([display](docs/design/V3_DISPLAY.md)).
- **Multi-GPU:** one kf3 device per host GPU. Two *distinct* host GPUs were measured in one guest,
  run one at a time and concurrently.
- **Driver matrix:** guests 575.57.08, 580.105.08, 580.159.04, 590.48.01, 595.84 and 610.57.04 pass
  the CUDA ladder; hosts 575.57.08, 580.65.06 and 580.95.05 pass the gates, the thin suite and the
  ladder ([driver matrix](docs/design/V3_DRIVER_MATRIX.md)).

Not working or not done:

- **Four apps need UVM demand paging** (managed memory faulted in on first touch). The host half
  is proven: an opt-in patch to the host's open nvidia-uvm delivers a real GPU fault to the owning
  process, which maps the page and replays it with correct data while ordinary host CUDA keeps
  working ([b3](docs/design/V3_UVM_B3_IMPLEMENTATION.md), tools only). Injecting the fault into the
  stock guest driver is in progress on branch `v3-uvm-guest`.
- **Performance depends on how often an app rings the doorbell.** llama.cpp (Qwen2.5-1.5B Q4_K_M,
  `llama-bench`) runs at **0.92× host decode and 0.96× host prefill** in the guest, on a nested box with
  every doorbell trapped: it submits a whole token's work at once, about 4,600 doorbells for ~200 tokens
  plus prefill (kf3 `4c48ca0c`, RTX 3060; `traces/v3_app_matrix/vast53004208_rtx3060_4c48ca0c/`,
  `m20/llama_bench.*`). The worst case is PyTorch eager on a 0.5B model, which launches every op
  separately: about 1,084 doorbells per token and **0.29×** host decode (**0.32×** with the opt-in doorbell
  fast path). On these nested boxes each doorbell costs the vCPU about 20 µs (15.7 µs with the fast path),
  about 22 ms of that model's ~58 ms per-token gap; the rest is not broken down yet. Non-nested hardware
  has not been measured for kayfabe. The C prototype of **this same design** — nvkvm Mode 2: the stock
  guest driver on an emulated GPU with every doorbell trapped, *not* the paravirtual nvkvm-pv — reached
  **1.05× host** llama.cpp decode on a bare-metal RTX 3050 (its copies ran on the CPU, so that result
  speaks to the doorbell cost, not the copy path). An optional guest doorbell module that removes the
  exit is designed, not built ([fast path](docs/design/V3_DOORBELL_IOEVENTFD.md),
  [module](docs/design/V3_GUEST_DOORBELL_MODULE.md)).
- **X11 desktops are partial:** they need a display class (`GF100_DISP_SW`) whose host policy awaits
  an owner decision.
- **Windows guests** are the last roadmap step. Only research exists.
- **Not yet measured:** two VMs sharing one GPU, and a fully rootless end-to-end boot. The design
  targets a host side that needs no root; the UVM patch is the one privileged piece, opt-in.

**Roadmap, in order (owner, 2026-09-26):** apps → the headless-graphics test set from nvkvm-pv →
display and a desktop (Linux Mint) → doorbell performance and Blackwell *(in parallel)* → the driver
matrix (535 → 610, every family) → Windows.

## How it compares

| | [nvkvm-pv](https://github.com/reindertpelsma/nvkvm-pv) | nvkvm Mode 2 (archive) | kayfabe v3 (this repo) |
|---|---|---|---|
| What it is | Shipped Mode-1 stack: a guest module forwards the driver's own API to the host | C research prototype that proved the emulated-GPU idea | Rust rewrite around a hostile-guest boundary |
| Guest kernel driver | Custom module you build and load | **Stock NVIDIA, unmodified** | **Stock NVIDIA, unmodified** |
| Guest OS | Linux only | Linux | Linux measured; Windows is the goal |
| GPUs run on hardware | Turing → Blackwell | GA106 | Turing, Ampere, Ada, Blackwell (TU116; GA102/104/106; AD104/106; GB203/205/206); Hopper source-derived only |
| CUDA / real apps | Yes | matmul, llama.cpp | `cup8` bit-exact; 61/65 apps |
| Graphics | Yes, incl. display | No | Headless Vulkan/EGL/GLX, bit-identical; display opt-in (Mint Cinnamon on Wayland) |
| Video engines | NVENC | No | NVENC/NVDEC, byte-identical |
| LLM decode vs host | 0.99–1.00× | ~parity, but the CPU copied the data | llama.cpp 0.92× (nested, trapped doorbells); PyTorch eager 0.5B 0.29× (0.32× with the fast path); bare metal not measured |
| Multi-tenant isolation | Not a security boundary | None | The design goal; two-VM sharing not yet measured |

The nvkvm-pv and archive columns are carried from earlier measurements and were not re-measured
for this page. If you want NVIDIA GPU forwarding that works today, use nvkvm-pv. Kayfabe is the bet
that you can do it without asking the guest to load your module at all.

## Requirements

| | |
|---|---|
| host | Linux x86_64 with `/dev/kvm`; ≥8 cores, ≥16 GB RAM, ≥100 GB free disk |
| GPU | Measured: Turing (TU116), Ampere (GA102, GA104, GA106), Ada (AD104, AD106), Blackwell (GB203, GB205, GB206). Hopper, GA100 and GB10x are derived from source only; GA100 and GB10B are refused |
| host driver | NVIDIA **open** kernel module **580.159.04** |
| hypervisor | QEMU **10.2.4** with the `kf3` overlay (`qemu/hw/misc/kf3`) compiled in, built by `scripts/bench/build_kf3.sh` |
| guest | Linux with the stock NVIDIA driver from the same `.run`. Guest RAM must be a shared memfd (`memory-backend-memfd,share=on`) |
| toolchain | stable Rust plus the `x86_64-unknown-linux-musl` target. `kayfabe-isolate-host` links a static helper, and without the target the whole workspace fails to build |

## Build and test

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --workspace
cargo test -p kf-abi -p kf-chip -p kf-rm -p kf-mem ...   # the kf-* crates; GPU-free
```

On a box with a GPU, in this order:

```sh
bash scripts/bench/v3_gates.sh                     # kf-gate1..9 on the real GPU, no QEMU: expect 9/9
bash scripts/bench/build_kf3.sh <qemu-10.2.4-src>  # installs <bench>/kf3-bins/<rev>/qemu-system-x86_64
cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder
bash scripts/fastguest/build_fast_guest.sh <guest.qcow2> <bench>/fastguest
KF_DEVICE=kf3 bash scripts/fastguest/fast_suite.sh <tag> 180   # 30 arms, one boot each: expect 30/30
```

A blank vast.ai box is provisioned by `scripts/bench/provision_box.sh`,
`provision_host_driver.sh` and `provision_bench_tree.sh`. The fat guest (CUDA ladder, apps, LLM,
graphics, video) boots through `scripts/bench/boot_capture.sh` with `KF_DEVICE=kf3`. Each lane has
its own script next to it: `cuda_ladder.sh`, `llm_parity.sh`, `gfx_suite.sh`, `video_lane.sh`.

## Repository layout

| path | what |
|---|---|
| `crates/kf-*` | **v3**, the product. `kf-trap` (vCPU trap path and doorbell plane), `kf-gsp` (faked GSP), `kf-rm` (emulated RM: RPC answers, object graph), `kf-chip` (family rows plus generated registers plus per-die facts from the host), `kf-abi` (generated NVIDIA ABI), `kf-host` (authored host RM verbs), `kf-mem` (the store and the walk diff), `kf-cuda` (the CUDA walk kernel), `kf-chan` (channels and completions), `kf-core`, `kf-qemu` (the `kf3-gpu` device's Rust half), `kf-harness` (the GPU gates), `kf-crec`/`kf-trace` (C-trace differential, oracle only), `kf-arch`, `kf-linux-raw`, `kf-util` |
| `crates/kayfabe-*` | The pre-v3 tree, **frozen**. It is kept for `kayfabe-rm-ladder`, the 30-arm raw-client grader, and the crates that grader depends on (`docs/design/V3_BUILD.md`, *Rules*) |
| `qemu/hw/misc/kf3/` | the C QOM device that `build_kf3.sh` lays into a QEMU tree |
| `cuda/walk/` | the page-table walk kernel (`.cu` plus committed PTX) |
| `scripts/bench/`, `scripts/fastguest/` | provisioning, gates, the fast and fat guest lanes |
| `traces/` | recorded reference captures and bench evidence |
| `third_party/` | pinned reference sources (submodules, not built): open-gpu-kernel-modules 580/610, Linux, gVisor |
| `archive/` | frozen: the C prototype, and the retired pre-v3 code |

Unsafe code is forbidden workspace-wide except in `kf-linux-raw`, `kf-qemu` and `kf-cuda` (and the
frozen `kayfabe-linux-raw`). In each of them, unsafe code lives only in files named `*_unsafe.rs`.

## Where to read more

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — v3 on one page: the planes, the crates, the rules.
- [`docs/STATUS_DETAIL.md`](docs/STATUS_DETAIL.md) — every status line above, with its
  revision, box and source doc.
- [`docs/design/THE_ARCHITECTURE_v3.md`](docs/design/THE_ARCHITECTURE_v3.md) (also
  [`docs/pdf/kayfabe_architecture_v3.pdf`](docs/pdf/kayfabe_architecture_v3.pdf)),
  [`THE_V3_PLAN.md`](docs/design/THE_V3_PLAN.md), and
  [`THE_CONSTRAINTS.md`](docs/design/THE_CONSTRAINTS.md) — the design, the build plan, and the
  owner's constraints.
- `docs/design/V3_*.md` — one document per v3 subsystem or result, each opening with a dated
  STATUS line.
- [`docs/PRODUCT_POSITIONING.md`](docs/PRODUCT_POSITIONING.md) — what kayfabe is for and who
  it is for.
- [`docs/PROJECT_HISTORY.md`](docs/PROJECT_HISTORY.md) — Mode 1 vs Mode 2, the C prototype,
  nvkvm-pv, and the corrections log.
- `docs/whitepaper/` describes the **pre-v3** architecture. It is kept as a record.

## Licence

Apache License 2.0. See [`LICENSE`](LICENSE); it applies to the whole repository, including
`archive/`.
