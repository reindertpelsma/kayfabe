# kayfabe

Kayfabe emulates an NVIDIA GPU inside QEMU. The guest sees what looks like real hardware —
an emulated device plus a faked GSP — and loads the **real, unmodified NVIDIA driver**
against it. Nothing in the guest is patched or shimmed. Kayfabe recovers what the guest is
actually trying to compute from its own protocol (RM allocations, page-directory binds,
doorbells, pushbuffer methods) and forwards that work to a real GPU on the host, from an
**unprivileged** host process. Two properties fall out: the guest OS stops being a
constraint (Windows guests are the end goal, because you can't ask Windows to load your
kernel module but you can let it load NVIDIA's), and the guest's driver version is
decoupled from the host's.

It is written in Rust, and it is a clean-slate rewrite of a C research prototype kept here
at [`archive/nvkvm/`](archive/README.md).

## Status

**Research stage. Published early so the approach can be read and argued with, not because
it is usable.** There is no supported build, no install path, and no stable entry point.
Interfaces, crate layout and on-disk formats change without notice.

Works today, measured on real hardware (GA106, host driver 580.159.04):

- A stock, unpatched guest NVIDIA driver boots against the emulated GPU + faked GSP.
- First compute: `cup3` (context create → kernel launch) returns the un-forgeable value.
- Matmul at scale: `cup8`, N=2048, `bad=0 maxerr=0`; also N=3072 (36 MiB operands) × 12
  iterations.
- The host isolate runs unprivileged — empty capability sets, `NoNewPrivs`, non-root uid,
  witnessed by `scripts/bench/e0_isolate_witness.sh`.
- The same QEMU overlay compiles into and boots on QEMU 9.2.0 and 10.2.4.

Does not work today:

- **LLM and PyTorch workloads do not produce correct output.** The CUDA runtime initialises
  and `torch.cuda.is_available()` is True. One Qwen2 run has since emitted 16 tokens, but the
  text was garbage and the harness grades the token *count*, not the text — so treat this as
  not working until the grade checks the output.
- **Performance is far off native** — 22–81× for large kernels; small kernels are dominated
  by kayfabe's own doorbell handler.
- Multi-process results (two concurrent guest CUDA processes; three sequential ones) exist
  **only on a branch**, not on `master`, and the 4th sequential process fails.
- **Multi-tenancy is the thesis, not a result.** There is no tenant axis in the code and no
  two-VM run has ever been attempted.
- `cargo test --workspace --no-fail-fast` does not pass clean: 2949 pass, 9 fail across 258
  test binaries (measured 2026-09-06). The failures are adjudicated and deliberate — they
  are red because a design ruling is outstanding, and they must not be edited to pass.

The detail behind every line above, with dates and revisions, is in
[`docs/STATUS_DETAIL.md`](docs/STATUS_DETAIL.md), and
[`docs/PROJECT_HISTORY.md`](docs/PROJECT_HISTORY.md) covers how the three projects relate.

## How it compares

Three related things exist. They are not competitors — the first is what to use, the second is
where the idea was proven, the third is this repo.

| | [nvkvm-pv](https://github.com/reindertpelsma/nvkvm-pv) | nvkvm Mode 2 (archive) | kayfabe (this repo) |
|---|---|---|---|
| What it is | Shipped Mode-1 stack: a guest module forwards the driver's own API to the host | C research prototype that proved the emulated-GPU idea | Clean-slate Rust rewrite of that prototype |
| Guest kernel driver | Custom module you build and load | **Stock NVIDIA, unmodified** | **Stock NVIDIA, unmodified** |
| Guest OS | Linux only | Linux | Linux tested; Windows is the point of the design |
| Guest/host driver versions | Must match | Decoupled | Decoupled by design |
| GPUs covered | Turing → Blackwell, six architectures, drivers 535–610 | One (GA106) | One (GA106) |
| CUDA compute | Yes | Yes — matmul N=1024, bit-exact | Yes — matmul N=2048, bit-exact |
| LLM inference | Yes — 32B via vLLM at 0.99–1.00× host, token-identical at temperature 0 | Yes — llama.cpp at 49.9 tok/s vs 47.5 host-native | Not correct yet (see above) |
| Graphics / Vulkan / display | Yes | No | No |
| Several CUDA processes in one guest | Yes | One per VM lifetime | Branch only; the 4th sequential one fails |
| Performance vs host | 98–100% on most workloads; three shapes cost more | Comparable on its one workload | 22–81× off |
| Multi-tenant isolation | Explicitly not a security boundary yet | No isolation vocabulary at all | The reason the rewrite exists — designed for, not yet demonstrated |
| Status | Works today, maintained | Archived, superseded by this | Research |

Two caveats worth stating plainly, because the numbers above are easy to misread:

- The archive's tok/s looks like parity, and in a sense it was — but its data plane copied
  through the CPU rather than the GPU's copy engines. It is a fair measure of that prototype
  and not a forwarding baseline.
- The archive had no notion of refusing anything, and no distinction between channels it may
  inspect and channels it may not. That is precisely the boundary this rewrite is built
  around, and it is why "rewrite" rather than "port".

If you want NVIDIA GPU forwarding that works today, use nvkvm-pv. Kayfabe is the bet that you
can do it without asking the guest to load your module at all.

## Requirements

For the logic-only build and test suite: a stable Rust toolchain and the
`x86_64-unknown-linux-musl` target (`kayfabe-isolate-host`'s `build.rs` links the isolate
binary statically against musl; without the target the *whole workspace* fails to build with
a confusing `can't find crate for std` on an unrelated crate).

For a run against a real GPU, as the bench provisioning scripts establish them
(`scripts/bench/host_preflight.sh`, `provision_box.sh`, `provision_host_driver.sh`,
`provision_bench_tree.sh`):

| | |
|---|---|
| host | Linux x86_64 with `/dev/kvm` and `vmx`/`svm`; ≥8 cores, ≥16 GB RAM, ≥100 GB free disk |
| GPU | NVIDIA **GA10x / Ampere** (RTX 30-series, A4000/A5000/A10). The bench is a GA106. Other generations are not oracle-checked in `kayfabe-arch` |
| host driver | NVIDIA **open** kernel module, pinned at **580.159.04**. The closed module is not what the oracles were built against |
| hypervisor | QEMU **10.2.4** with this repo's QOM overlay compiled in (9.2.0 also verified). There is no out-of-tree device mechanism, so the hypervisor is built once |
| guest | Ubuntu Noble cloud image with the stock NVIDIA driver installed from the same `.run`. `-cpu host` is required |

## Build

```sh
rustup target add x86_64-unknown-linux-musl

cargo build --workspace
cargo test  --workspace --no-fail-fast   # see Status: 9 known failures
cargo clippy --all-targets

KAYFABE_SLOW=1 cargo test --workspace    # + the measured-slow tests
scripts/run_full_suite.sh --list         # everything, on a real box, and what it skipped
```

`cargo test --workspace` **without** `--no-fail-fast` stops at the first failing target and
reports a stopping point that looks like a result. GitHub CI is opportunistic convenience;
`scripts/run_full_suite.sh` on real hardware is the authoritative run.

## Running against a real GPU

Two gates are **off by default**, and missing either produces a refusal that does not
obviously name the cause:

1. **Build feature `host-isolates`** on `kayfabe-qemu-raw`. Without it the archive cannot
   even name the real host isolate factory, so the forwarding plane is absent by linkage.
   `scripts/build_qom_shim.sh` takes it via `KAYFABE_SHIM_FEATURES`.
2. **Runtime `KAYFABE_ISOLATES=real`**. The feature is not the selector. Valid values are
   `stillborn` (the default — every isolate is retired at birth), `loopback` and `real`;
   there is no default-to-real, because a typo that silently selected the refusing plane
   would make an evidence run and its own negative control indistinguishable.

```sh
# 1. build the Rust archive and lay the QOM overlay into a QEMU source tree
KAYFABE_SHIM_FEATURES=host-isolates scripts/build_qom_shim.sh /path/to/qemu-10.2.4

# 2. boot the guest against the emulated device
KAYFABE_ISOLATES=real qemu-system-x86_64 \
  -machine q35,accel=kvm -cpu host -m 2048 \
  -device nvkvm-gpu,bar1-size=268435456,bar2-size=33554432,id=kf0 \
  ...
```

`scripts/bench/boot_nvkvm.sh` is the boot line actually used, and
[`docs/reference/bench_rebuild_notes.md`](docs/reference/bench_rebuild_notes.md) is the
first-person log of standing a bench up from a blank box — read it before rebuilding one.

## Repository layout

23 crates plus the conformance suite. Descriptions are the crates' own; per-layer state is
in [`docs/STATUS_DETAIL.md`](docs/STATUS_DETAIL.md).

| crate | layer | |
|---|---|---|
| `kayfabe-core` | L0 | composition root: the `RmGraph` source of truth and the ownership spine |
| `kayfabe-mmu` | L0 | address plane: per-VAS address table (the guest's TLB) and the GMMU walk |
| `kayfabe-completion` | L0 | per-process completion engine: pending sets, drain-gated batching |
| `kayfabe-fwd` | L0 | intent recovery to host ops: doorbell demux, VAS materialization |
| `kayfabe-gsp` | L0 | the faked GSP: falcon boot FSM, message queues, RPC codec |
| `kayfabe-rmrpc` | L0 | GSP→core bridge: one decoded RPC becomes one `RmEvent`, stateless and pure |
| `kayfabe-device` | L0 | emulated device: chip table, register plane, PCI routing |
| `kayfabe-abi` | L0 | codegen'd NVIDIA ABI structs — the only crate holding `#[repr(C)]` wire types |
| `kayfabe-arch` | L0 | abstract GPU vocabulary and the `Arch` trait set (cross-generation) |
| `kayfabe-chips` | L0 | per-generation `Arch` impls: GA10x, Ada, and Hopper as a refutation fixture |
| `kayfabe-vmm` | port | hypervisor-adapter port: `Vmm`, `Device`, `Present` |
| `kayfabe-isolate` | port | per-process sandbox port: `Isolate`, `IsolateFactory`, `RmBackend` |
| `kayfabe-rt` | L1 | threaded shell: ranked-lock discipline, executor inbox |
| `kayfabe-shell` | L1 | OS shell: reactor loop, descriptor registrar, executor thread |
| `kayfabe-linux-raw` | L1 | audited raw-OS adapter — one of the two crates permitted `unsafe` |
| `kayfabe-isolate-host` | L1 | the sandboxed child process that issues the real NVIDIA RM ioctls |
| `kayfabe-vmm-qemu` | L2 | QEMU adapter logic: `Vmm` impl, guest-physical map, region classification |
| `kayfabe-qemu-raw` | L2 | QEMU FFI surface: `extern "C"` entry points — the second audited `unsafe` crate |
| `kayfabe-vmm-kvm` | L2 | KVM-direct adapter: real VM descriptors, memslots, mmap'd backings |
| `kayfabe-trace` | — | structured trace events, budgets, replay format |
| `kayfabe-crec` | — | C↔Rust trace decoder and divergence classifier |
| `kayfabe-mocks` | — | deterministic in-process mock adapters for GPU-free testing |
| `kayfabe-util` | — | generic utilities, no GPU concepts |
| `tests/` | — | the conformance suite and the `Scenario` DSL |

Also: `qemu/hw/misc/nvkvm/` (the C QOM shim overlay), `scripts/` (build, bench and gate
scripts), `traces/` (recorded reference captures), `archive/nvkvm/` (the frozen C
prototype), `fuzz/` (its own workspace — the only place unsafe dependencies are allowed).

## Where to read more

- [`docs/whitepaper/kayfabe_architecture.pdf`](docs/whitepaper/kayfabe_architecture.pdf) —
  **the architecture paper. Start here.** Written to be attacked; roughly half of it is about
  what does not work, is not built, or is not known.
- [`ARCHITECTURE.md`](ARCHITECTURE.md) — the hexagonal core, its ports, and the crate map.
- [`docs/STATUS_DETAIL.md`](docs/STATUS_DETAIL.md) — long-form status with dates, revisions
  and the corrections behind each claim.
- [`docs/PROJECT_HISTORY.md`](docs/PROJECT_HISTORY.md) — Mode 1 vs Mode 2, the C prototype,
  the relationship to nvkvm-pv.
- [`docs/design/`](docs/design/) — the settled designs. `core_state_and_consolidation.md`
  for L0; `l1_concurrency.md` and `l1_os_shell.md` for L1 (read their contact logs);
  `l2_qemu_adapter.md` for the QEMU overlay; `execution_plane.md`,
  `core_security_threat_model.md`, `testing_doctrine.md`, `claim_ledger.md`.
- [`docs/reference/`](docs/reference/) — what has been measured on real hardware, kept
  separate from design so a wrong fact is corrected once.
- [`CLAUDE.md`](CLAUDE.md) — repository conventions.

## Licence

Apache License 2.0. See [`LICENSE`](LICENSE) at the repository root; it applies to the whole
repository, including the frozen C prototype under `archive/nvkvm/`.
