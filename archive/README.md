# archive

Frozen reference material. Nothing here is built, tested, maintained, or
intended to run.

## `nvkvm/`

The original **nvkvm** C research prototype, snapshotted at commit `bac00b6`
and imported as a single squashed commit (its own history was not carried
over).

It is kept because kayfabe's design references its **Mode-2** work directly —
the differential oracle, the address table, the forwarding model, and lessons
#11–#14. When a kayfabe design doc says "the C artifact does X", this is the
code it means.

Two things it is not:

- **Not a supported project.** It does not build here, is not covered by CI,
  and will not be fixed.
- **Not the maintained descendant.** That is
  [nvkvm-pv](https://github.com/reindertpelsma/nvkvm-pv), which was forked from
  this prototype and deliberately **excludes** Mode 2 — Mode 2 is a research
  artifact, and nvkvm-pv ships only the paths that are tested.

## The pre-v3 kayfabe tree (archived 2026-09-26)

v3 — the `kf-*` crates and the `kf3` QEMU device (`crates/kf-qemu` +
`qemu/hw/misc/kf3`) — is kayfabe's architecture (`docs/design/THE_V3_PLAN.md`).
The first Rust port it replaces is kept here for provenance: many comments and
design docs cite it by path. **A citation of `crates/kayfabe-X/...` for one of
the crates below now resolves to `archive/crates/kayfabe-X/...`**, and
`tests/...` / `fuzz/...` to `archive/tests/...` / `archive/fuzz/...`.

Nothing here is built (`exclude = ["archive"]` in the root `Cargo.toml`), tested
or maintained. History was preserved (`git mv`), so `git log --follow` works.

| path | what it was |
|---|---|
| `crates/kayfabe-core`, `-fwd`, `-rmrpc`, `-rt`, `-shell` | the old pure core, forwarding plane, RM-RPC bridge and L1 threaded shell |
| `crates/kayfabe-qemu-raw`, `-vmm-qemu`, `-vmm-kvm` | the old `nvkvm-gpu` QEMU device and VMM adapters |
| `crates/kayfabe-crec` | the old C-reference replay (v3's is `crates/kf-crec`) |
| `tests/` | `kayfabe-tests`, the old conformance suite |
| `fuzz/` | `kayfabe-fuzz`, cargo-fuzz targets over the old core |
| `qemu/hw/misc/nvkvm/` | the old device's C QOM overlay |
| `scripts/build_qom_shim.sh` | built QEMU with the old device |
| `scripts/run_full_suite.sh`, `orphan_gate.sh`, `compat_matrix.py`, `bite_*.py` | old-tree suite runner, gates and falsifiers |
| `scripts/advguest/`, `scripts/bench/w*`, `runners/`, `single_store_e*_boot.sh`, … | per-experiment runners of the pre-v3 campaign (w263–w754), all booting the old device |

### What was deliberately NOT archived

Nine `kayfabe-*` crates stay in `crates/`: `kayfabe-rm-ladder` — the 30-arm
raw-client grader behind the thin-guest suite (`scripts/fastguest/`) and the
bare-metal baseline — and its dependency closure (`abi`, `arch`, `chips`,
`isolate`, `isolate-host`, `linux-raw`, `util`, `vmm`). Moving the grader onto
`kf-*` crates would let them follow.

## The grader's dead weight (archived 2026-10-15)

Last commit where all of the following was still in `crates/`: `6fb32dfb8`
(`git show 6fb32dfb8:crates/<crate>/...`). The test for every move below was the
same: the grader builds (`cargo build -p kayfabe-rm-ladder --bins`,
`-p kayfabe-isolate-host --bins`) and every test target of a kept crate still
compiles (`cargo test --workspace --no-run`). History was preserved (`git mv`).

| path | what it was |
|---|---|
| `crates/kayfabe-device`, `-mmu`, `-trace`, `-gsp`, `-completion`, `-mocks` | the old register plane / device model, MMU walker, trace recorder, GSP state machine, completion model and mock backends. The grader listed `kayfabe-device` and `-mmu` as dependencies and never used them in code |
| `crates/kayfabe-doorbell` | the hash-frozen doorbell-plane model (`scripts/ci/frozen-debt.json`, now gone). Its only live consumer was `classgen` (the generated per-family class tables), which now lives in `crates/kayfabe-chips/src/classgen.rs` and is **not** in the archived copy |
| `crates/kayfabe-cuda`, `cuda/walk-v2-frozen/` (now `archive/cuda/walk-v2-frozen/`) | the old walk-kernel binding and its frozen PTX. Only the opt-in `cuda-scratchpad` / `cuda-window` features used it (no script passed them); v3's is `kf-cuda` |
| `prev3-modules/kayfabe-chips/` | `ad10x`, `ga10x`, `gb20x`, `gh100` (per-generation `Arch`/`GspModel` adapters) with their tests; the three `host_classes` tests that cross-check those adapters are in `tests/host_classes_arch_crosscheck.rs` |
| `prev3-modules/kayfabe-abi/` | 33 of 49 modules (per-control wire tables and info encoders) and two tests |
| `prev3-modules/kayfabe-linux-raw/` | `arena_unsafe`, `memtype`, `procfd`, `tests/effective_memtype.rs` |
| `prev3-modules/kayfabe-isolate-host/` | `planreactor`, `cudastore`, `cudawalk`, `tests/plan_reactor.rs` |

`prev3-modules/<crate>/` mirrors the crate's own layout (`src/`, `tests/`). The
modules are not a crate and are not built. The citation rule above applies:
`crates/kayfabe-device/...` now resolves to `archive/crates/kayfabe-device/...`,
and a moved module `crates/kayfabe-abi/src/X.rs` to
`archive/prev3-modules/kayfabe-abi/src/X.rs`.

The bench defaults to `KF_DEVICE=kf3`; `KF_DEVICE=nvkvm` refuses by name.
