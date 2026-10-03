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

**Research stage:** no install path, no stable interface. Results come from rented vast.ai boxes that are
themselves VMs, so the guest under test is *nested*. Every line below, with its revision, box and
evidence, is in [`docs/STATUS_DETAIL.md`](docs/STATUS_DETAIL.md) §0; the resume point is
[`docs/STATUS_AND_HANDOFF.md`](docs/STATUS_AND_HANDOFF.md).

Works, measured:

- **The stock NVIDIA driver runs on every family from Turing to Blackwell** (TU116, GA102/104/106,
  AD104/106, GB203/205/206). The current code passes 1,754 tests, the nine GPU gates, and the 30-arm
  client suite both on bare metal and in the guest (30/30).
- **CUDA is correct** (a 2048² matmul is bit-exact), and **61 of 65 real applications pass**: PyTorch,
  Hugging Face, llama.cpp, CuPy, hashcat, Blender, Geekbench and the CUDA samples, including dynamic
  parallelism.
- **llama.cpp runs at 0.92× host decode** (0.96× prefill), even on a nested box.
- **Headless graphics passes nvkvm-pv's test set plus 15 more items (Vulkan, EGL, GLX: 38/38)**, 31 of
  them byte-identical to bare metal; NVENC/NVDEC output is byte-identical too.
- **Display (opt-in):** a Linux Mint Cinnamon desktop on Wayland, weston, and Xorg with the NVIDIA
  driver, on an emulated monitor.
- **Multi-GPU**, and a range of driver versions: guests 575 to 610, hosts 575 to 580.

Not yet:

- **Four apps need UVM demand paging.** The host side is proven; the guest side is in progress.
- **Apps that launch every kernel separately are slow on nested hosts** (PyTorch eager on a small
  model: 0.29× host), because each launch traps into the VMM. Bare metal is not measured yet.
- X11 desktops are partial; Windows guests are the last roadmap step; two VMs sharing one GPU is not
  measured.

**Roadmap (owner, 2026-09-26):** apps → nvkvm-pv's headless-graphics tests → display and a desktop →
doorbell performance and Blackwell → the driver matrix (535 → 610, every family) → Windows.

## How it compares

| | [nvkvm-pv](https://github.com/reindertpelsma/nvkvm-pv) | kayfabe v3 (this repo) | nvkvm Mode 2 (archive) |
|---|---|---|---|
| What it is | Shipped Mode-1 stack: a guest module forwards the driver's own API to the host | Rust rewrite around a hostile-guest boundary | C research prototype that proved the emulated-GPU idea |
| Guest kernel driver | Custom module you build and load | **Stock NVIDIA, unmodified** | **Stock NVIDIA, unmodified** |
| Guest OS | Linux only | Linux measured; Windows is the goal | Linux |
| GPUs run on hardware | Turing → Blackwell | Turing → Blackwell (Hopper source-derived only) | GA106 |
| CUDA / real apps | Yes | `cup8` bit-exact; 61/65 apps | matmul, llama.cpp |
| Graphics | Yes, incl. display | Headless Vulkan/EGL/GLX bit-identical; display opt-in (Mint desktop) | No |
| Video engines | NVENC | NVENC/NVDEC, byte-identical | No |
| LLM decode vs host | 0.99–1.00× | llama.cpp 0.92× (nested); PyTorch eager 0.29× | ~parity (bare metal), but the CPU copied the data |
| Multi-tenant isolation | Not a security boundary | The design goal; two-VM sharing not yet measured | None |

The nvkvm-pv and archive columns are carried from earlier measurements and were not re-measured
for this page. If you want NVIDIA GPU forwarding that works today, use nvkvm-pv. Kayfabe is the bet
that you can do it without asking the guest to load your module at all.

## Requirements

| | |
|---|---|
| host | Linux x86_64 with `/dev/kvm`; ≥8 cores, ≥16 GB RAM, ≥100 GB free disk |
| GPU | Turing through Blackwell (measured dies: [`STATUS_DETAIL.md`](docs/STATUS_DETAIL.md) §0). Hopper, GA100 and GB10x are derived from source only; GA100 and GB10B are refused |
| host driver | NVIDIA **open** kernel module **580.159.04** |
| hypervisor | QEMU **10.2.4** with the `kf3` overlay (`qemu/hw/misc/kf3`) compiled in, built by `scripts/bench/build_kf3.sh` |
| guest | Linux with the stock NVIDIA driver from the same `.run`. Guest RAM must be a shared memfd (`memory-backend-memfd,share=on`) |
| toolchain | Rust **1.99.0**, pinned in `rust-toolchain.toml` (rustup picks it up), with the `x86_64-unknown-linux-musl` and `x86_64-unknown-uefi` targets listed there too. `kayfabe-isolate-host` links a static helper, and `kf-gop-image` builds kf3's boot-display UEFI driver from `firmware/kf-gop` (nothing compiled is committed); without either target the workspace fails to build, with an error naming it |

## Build and test

```sh
rustup target add x86_64-unknown-linux-musl x86_64-unknown-uefi
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

Dual-licensed under **Apache-2.0 OR GPL-2.0-or-later**, at your option: the full texts are
[`LICENSE-APACHE`](LICENSE-APACHE) and [`LICENSE-GPL`](LICENSE-GPL), and [`LICENSE`](LICENSE) states
the grant and its exceptions. The exceptions are the `third_party/` submodules, files that carry their own
notice, third-party data inside recorded traces, and the files derived from gVisor's nvproxy. The
nvproxy-derived files stay Apache-2.0 only until they are re-derived. Because the kf3 QEMU device links
one of them, a QEMU binary built from this tree is not yet distributable under the GPL.
