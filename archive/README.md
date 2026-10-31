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
| `crates/kayfabe-device` (moved 2026-10-31) | the old register-plane device, 49 734 lines; see the STATUS note below |
| `tests/` | `kayfabe-tests`, the old conformance suite |
| `fuzz/` | `kayfabe-fuzz`, cargo-fuzz targets over the old core |
| `qemu/hw/misc/nvkvm/` | the old device's C QOM overlay |
| `scripts/build_qom_shim.sh` | built QEMU with the old device |
| `scripts/run_full_suite.sh`, `orphan_gate.sh`, `compat_matrix.py`, `bite_*.py` | old-tree suite runner, gates and falsifiers |
| `scripts/advguest/`, `scripts/bench/w*`, `runners/`, `single_store_e*_boot.sh`, … | per-experiment runners of the pre-v3 campaign (w263–w754), all booting the old device |

### What was deliberately NOT archived

**STATUS: LIVE, 2026-10-31.** ⊘ Correction, folded above the text it corrects: `kayfabe-device`
(49 734 lines) is no longer in this list of kept crates. It was kept on 2026-09-26 only because
`kayfabe-rm-ladder` declared a dependency on it; the ladder's source never names it, and
`cargo tree -p kayfabe-rm-ladder` shows nothing else in the closure uses it. The declaration was
removed and the crate moved to `archive/crates/kayfabe-device`, which is **not a workspace member
and no longer builds or tests**. It last built at `3e51cfcde` (tag `pre-archive-20261031`, the
`next` tip it was moved from). Its tests were also dropped from `cargo test --workspace`; the
clippy-debt entries for it were removed, and its claim-ledger debt entries were re-keyed to the new path.
The kf-abi and kayfabe-abi truncated-row gates still read its files through
`archive/crates/kayfabe-device`.

**STATUS: LIVE, 2026-10-31 (second pass).** ⊘ Refinement, folded above the text it refines: six of
the sixteen crates below link **0 bytes** into the release `kayfabe-rm-ladder` (MEASURED, `nm -S -C`,
`docs/design/V3_RAW_CLIENT.md` §2.2): `mmu`, `gsp`, `mocks`, `trace`, `completion`, `cuda`. Zero linked
bytes is not zero use: five of the six are **compiled** unconditionally by a live crate and `cuda` under the ladder's `cuda-window` feature,
so, unlike `kayfabe-device`, none of them is an unused declaration.
The chain is `kayfabe-isolate-host` -> `kayfabe-chips` -> {`kayfabe-gsp` (`ad10x.rs` uses
`FalconSecureBooterBoot`), `kayfabe-mocks` (`MockArch` in `ad10x`/`gb20x`/`gh100`)};
`kayfabe-gsp` -> `kayfabe-trace` (`replay.rs`) -> {`kayfabe-completion`, `kayfabe-mmu`
(`event.rs`)}; `kayfabe-cuda` is `kayfabe-isolate-host`'s optional `cuda-scratchpad` dependency
(`cudawalk.rs`, `cudastore.rs`, `rm.rs`), reached from the ladder's opt-in `cuda-window` feature, so
`--all-features` builds it. The ladder itself and `isolate-host` use **only** `chips::host_classes`
(`DerivedHostClasses`, `pinned_host_classes`, `CLASSLIST_V2_SIZE`, `decode_classlist`); nothing live uses
`ga10x`, `ad10x`, `gb20x` or `gh100`. Those four modules are what pull in the five crates, so the five can
follow only after `kayfabe-chips` is cut down to `host_classes`, which is an owner decision (it edits a
frozen live crate, and `kayfabe-mmu`'s tests, which use `chips::ga10x::Ga10xGmmu` for the `cuda/walk`
differential oracle, would go with it). The ladder's declarations of `kayfabe-chips` and `kayfabe-mmu` were removed on
2026-10-31 (its `main.rs` names neither; they stay in the closure through `isolate-host`). Sub-2 KB crates
(MEASURED symbol bytes): `chips` 1,590 (the class-list decoder), `doorbell` 1,111 (the generated
`classgen::FAMILIES` table), `arch` 167 (`ids`), `vmm` 46 (the two-variant `Prot` enum). `vmm` is a
single-enum inline candidate (its live users are `kayfabe-isolate` and `isolate-host`); none was moved.

Sixteen `kayfabe-*` crates stay in `crates/`: `kayfabe-rm-ladder` — the 30-arm raw-client grader behind the
thin-guest suite (`scripts/fastguest/`) and the bare-metal baseline — and the dependency closure it
actually has (`abi`, `arch`, `chips`, `completion`, `cuda`, `doorbell`, `gsp`, `isolate`,
`isolate-host`, `linux-raw`, `mmu`, `mocks`, `trace`, `util`, `vmm`). Every one of them is reached
from `kayfabe-rm-ladder` through normal dependencies (`cargo tree`), so none of them can move until
the grader is rebuilt on `kf-*` crates, which would let them follow.

The bench defaults to `KF_DEVICE=kf3`; `KF_DEVICE=nvkvm` refuses by name.
