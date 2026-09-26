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

## Status — 2026-09-26, `master` at `74dc3113`

**Research stage, published so the approach can be read and argued with.** There is no install
path and no stable interface. Every result below was measured on rented vast.ai boxes that are
themselves KVM guests (so the guest under test is *nested*), host driver 580.159.04 (open).
Sources are in [`docs/STATUS_DETAIL.md`](docs/STATUS_DETAIL.md).

Works, measured:

- **Stock driver boots and passes the 30-arm thin-guest suite, 30/30,** on **GA106** (RTX 3060)
  and **AD106** (RTX 4060 Ti). Bare metal on the same boxes is also 30/30, so a guest failure
  indicts kayfabe, not the test client. The nine GPU harness gates (`kf-gate1`…`9`, no QEMU)
  pass 9/9.
- **CUDA is correct:** `cup3` returns 43; `cup8` (2048² matmul) returns `BAD=0 MAXERR=0`.
- **Real applications: 58 of 65 pass** in the guest (host: 65/65) at `670bd310`. These include
  PyTorch (a CNN training step with the same digest as the host), Hugging Face generate,
  llama.cpp (tokens identical to the host), CuPy, hashcat, Blender CUDA+OptiX, Geekbench, and
  the CUDA samples, with non-default streams and CUDA graphs. The matrix is in
  `docs/design/V3_APP_MATRIX.md`, on branch `v3-apps2`. Two problems it names are fixed on
  `master`: `gpu_burn` crashing, and a host BAR1 leak that stopped a boot at about 60 CUDA
  processes. The full matrix has not been re-run since.
- **Headless graphics renders the same as bare metal, bit for bit:** Vulkan, EGL, and GLX
  through Xvfb + VirtualGL. `nvidia-drm modeset=1` registers its render node in displayless
  mode.
- **NVENC and NVDEC output is byte-identical** to bare metal.
- **Several CUDA processes run in one guest.** 100 sequential processes ran in one boot with
  guest persistence mode.
- **Multi-GPU:** one kf3 device per host GPU. Two *distinct* host GPUs were measured in one
  guest, run one at a time and concurrently. Asking for the same card twice is refused by name
  when its BAR1 budget does not fit.

Not working or not done:

- **Six apps still fail.** Five of them need **UVM demand paging**: managed memory touched
  first by the CPU or GPU, and kernels that access pageable host memory. The guest's UVM expects
  a replayable fault; kayfabe delivers none, and the host RM kills the channel group with Xid 31.
  The guest *is* told: CUDA returns 719. One sample still prints `SUCCESS` because it checks
  neither the error nor its result. The fix appears to need a privileged host piece; the
  research is on branch `v3-uvm-research`. The sixth failure is a host map that kayfabe refuses
  (`UnifiedMemoryStreams`), being worked on in branch `v3-mapfix`.
- **Performance.** LLM decode runs at **0.29–0.31×** the same box's host tok/s, with guest text
  identical to host text. The whole gap is doorbells: about 1,080 per token, each a trapped VM
  exit, which costs about 51 µs on a nested box. An optional guest doorbell module that removes
  the exit is **designed, not built** (`docs/design/V3_GUEST_DOORBELL_MODULE.md`).
- **Other GPU families have not been run on hardware.** Turing has a GSP model; Hopper and
  Blackwell are derived from the open driver's source. None of the three has booted. GA100 is
  refused by name.
- **Display is not started.** There is no scanout and no Xorg with NVIDIA's own display driver.
  Headless rendering (above) works.
- **Windows guests** are the last roadmap step. Only research exists.
- **Not yet measured:** two VMs sharing one GPU, and a fully rootless end-to-end boot. The host
  side uses only RM controls that the host driver allows unprivileged clients to call.
- GitHub CI is red on `master`. The verdict of record is the hardware run: `v3_gates.sh`, then
  the fast suite (see *Build and test*).

**Roadmap, in order (owner, 2026-09-26):** apps → the headless-graphics test set from nvkvm-pv →
display and a desktop (Linux Mint) → doorbell-module parity *(in parallel)* → Blackwell *(in
parallel)* → the guest-driver version matrix → Windows.

## How it compares

| | [nvkvm-pv](https://github.com/reindertpelsma/nvkvm-pv) | nvkvm Mode 2 (archive) | kayfabe v3 (this repo) |
|---|---|---|---|
| What it is | Shipped Mode-1 stack: a guest module forwards the driver's own API to the host | C research prototype that proved the emulated-GPU idea | Rust rewrite around a hostile-guest boundary |
| Guest kernel driver | Custom module you build and load | **Stock NVIDIA, unmodified** | **Stock NVIDIA, unmodified** |
| Guest OS | Linux only | Linux | Linux measured; Windows is the goal |
| GPUs run on hardware | Turing → Blackwell | GA106 | GA106, GA102, AD106; Turing/Hopper/Blackwell source-derived only |
| CUDA / real apps | Yes | matmul, llama.cpp | `cup8` bit-exact; 58/65 apps |
| Graphics | Yes, incl. display | No | Headless Vulkan/EGL/GLX, bit-identical; no display yet |
| Video engines | NVENC | No | NVENC/NVDEC, byte-identical |
| LLM decode vs host | 0.99–1.00× | ~parity, but the CPU copied the data | 0.29–0.31× (doorbell exits) |
| Multi-tenant isolation | Not a security boundary | None | The design goal; two-VM sharing not yet measured |

The nvkvm-pv and archive columns are carried from earlier measurements and were not re-measured
for this page. If you want NVIDIA GPU forwarding that works today, use nvkvm-pv. Kayfabe is the bet
that you can do it without asking the guest to load your module at all.

## Requirements

| | |
|---|---|
| host | Linux x86_64 with `/dev/kvm`; ≥8 cores, ≥16 GB RAM, ≥100 GB free disk |
| GPU | Measured: GA10x (GA106, GA102) and Ada (AD106). Turing, Hopper and Blackwell are derived from source and untested. GA100 is refused |
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
