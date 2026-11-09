# Contributing to kayfabe

**STATUS: LIVE, 2026-11-07.** kayfabe is a research project with one maintainer. Contributions are
welcome, and reviews can be slow. This page is the short version; the rules that bind the work are in
[`AGENTS.md`](AGENTS.md), and the design is in [`ARCHITECTURE.md`](ARCHITECTURE.md) and `docs/design/`.

## What you need

This repository alone is enough to build and run the GPU-free tests. You do not need a GPU, a private
repository or an account anywhere. The submodules in `third_party/` are reference sources: nothing is
built from them, and a normal clone does not fetch them.

```sh
git clone https://github.com/reindertpelsma/kayfabe.git && cd kayfabe
rustup toolchain install 1.99.0 --profile minimal      # pinned in rust-toolchain.toml
rustup target add --toolchain 1.99.0 x86_64-unknown-linux-musl x86_64-unknown-uefi
cargo build --workspace
cargo test -p kf-oprom                                  # any kf-* crate; no GPU needed
```

The GPU-free test suite is `KAYFABE_NO_KVM=1 cargo test --workspace --no-fail-fast` (what the `CI`
workflow runs); `cargo test -p kf-<crate>` runs one crate. The `scripts/ci/` gates run as
`python3 -m unittest discover -s scripts/ci -p 'test_*.py'` plus the scripts named in
`scripts/ci/README.md` (doc freshness, claim ledger, private references, GPU UUIDs).

The package list, the QEMU build and the extra targets are in
[`docs/howto/building.md`](docs/howto/building.md). Tests that need a reference tree (an
NVIDIA open-gpu-kernel-modules checkout, say) say `SKIP` on stderr when it is missing; that is not a
pass. To run them, fetch the tree you need:
`git submodule update --init --depth 1 third_party/ogkm`.

Anything that runs on a real GPU (the hardware gates, the guest suites) needs an NVIDIA GPU and
the open NVIDIA host driver; see [`docs/howto/requirements.md`](docs/howto/requirements.md). You do not
need to run those for a documentation or GPU-free change, and CI does not either, except on the batch
pull request into `master`.

## Cloning a lean copy

A full checkout is about 0.7 GB in the working tree, mostly committed evidence under `traces/`, and
the history behind `next` is a few hundred MB more. Most of it is not needed to build, test or review
code. What does **not** help: `git clone --filter=blob:limit=1m --depth=1` works, but git then fetches
every blob it needs to write the working tree, so the result is as large as a plain `--depth=1` clone.
What does help is a **sparse checkout** of the paths the code reads. The first part of this list is
the one the CI Worker's boxes use (`SPARSE` in `ci/vast-broker/src/onstart.js`; if the two differ,
that file wins); the last eight lines add what two `kf-abi` test targets (`truncated_row_reads`,
`windows_cell_provenance`) read, which the Worker's list does not carry:

```sh
git clone --filter=blob:none --depth=1 --no-checkout --branch next https://github.com/reindertpelsma/kayfabe.git
cd kayfabe
git sparse-checkout set --no-cone --stdin <<'LIST'
/*
!/archive/
/archive/nvkvm/tests/mode2/cup2.c
!/traces/
/traces/blackwell_consttab_20261023/generate_rows.py
/traces/blackwell_host_caps_review_20261022/tier_audit/precomp_fields.tsv
/traces/blackwell_hw_20261103/gb203_tier3/precomp_tier3.txt
/traces/blackwell_provider_validation_20261023/lease_metadata/precomp_tier3.txt
/traces/blackwell_windows_resume_20261023/lease_metadata/precomp_tier3.txt
/traces/c_oracle_census/
/traces/cap1_coldboot_hermetic.rec
/traces/cap1b_coldboot_hermetic_d6.rec
/traces/constants_measure_20261014b/
/traces/constants_night_20261014/
/traces/die_driver_sweep_20261015/
/traces/driver_matrix/
/traces/rawclient_ce_interrupt_20261008/
/traces/real_ad104/
/traces/real_ga104/
/traces/real_ga106/
/traces/real_hw_20261013/
/traces/real_tu116/
/traces/user_ring_scan_20261014/
/archive/crates/kayfabe-device/
/traces/windows_code43_walls_20261007/README.md
/traces/windows_code43_walls_20261007/run40-requests.log
/traces/windows_code43_walls_20261007/run41-requests.log
/traces/windows_code43_walls_20261007/run42-requests.log
/traces/windows_code43_walls_20261007/run53-command.json
/traces/windows_code43_walls_20261007/run53-requests.log
/traces/windows_pool_20261005/probe-b/qemu.log
LIST
git checkout
```

The excluded part is `archive/` (frozen, not built) and every other trace directory. A build or test
that says `couldn't read .../traces/<dir>/...` needs that directory: add `/traces/<dir>/` to the list
and run `git sparse-checkout set --no-cone --stdin` again with the longer list. Any evidence
directory can be added the same way.

MEASURED, 2026-11-07, by the agent `cleanup0`, from `origin/next` at `a9dee7b2c` over HTTPS; sizes
by `du -sm` in MiB (`.git` + working tree), wall time of the clone on one host. `git clone --depth=1
--branch next`: 270 + 701 = 971, 67 s. The same with `--filter=blob:limit=1m`: 268 + 701 = 969,
60 s. The sparse recipe above: 30 + 109 = 139, about 18 s. In the sparse tree `cargo test -p
kf-oprom -p kf-abi -p kf-crec --offline --locked` passed except 3 `kf-abi` tests (the two targets
named above) that failed with `No such file or directory` before the last eight lines were added;
those two targets passed after them.

## Sending a change

- **Open pull requests against `next`**, not `master`. `next` requires the `CI` check; `master` also
  requires the real-GPU check, which runs once on the batch pull request from `next`.
- One concern per pull request. Say what changed, why, and how you verified it. Keep what you
  measured apart from what you inferred, and name the revision a result was measured on.
- `crates/kf-*` is the product. `crates/kayfabe-*` is the frozen earlier tree: no new features there.
- Run `cargo fmt --all`, `cargo clippy` and the tests of the crates you touched. CI also runs the
  gates in `scripts/ci/` (`scripts/ci/README.md`).
- A change to what a user sees or does updates `docs/howto/` in the same pull request. Design notes
  go in `docs/design/` (each opens with a dated `STATUS` line), never in `docs/howto/`.
- Unsafe code lives only in `kf-linux-raw`, `kf-qemu` and `kf-cuda`, in files named `*_unsafe.rs`.

**Review checklist** (what a reviewer, and the CI gates, look for):

- The pull request bases on `next`, has one concern, and its description keeps what was measured
  apart from what was inferred, naming the run, the revision and the date.
- The `kf-*` crates you touched pass `cargo test -p`, `cargo clippy` and `cargo fmt --all --check`
  with no new warning. The clippy and claim ledgers only ratchet down: fix a finding, do not
  re-record the baseline.
- Every new or changed Markdown document opens with a dated `STATUS`
  (`python3 scripts/ci/doc_freshness.py`), and a word like measured or verified names its run
  (`python3 scripts/ci/debt.py claims`).
- The change keeps the rules of `AGENTS.md`: the guest is hostile, per-die facts are derived and never
  captured, only BAR0 writes trap, there is no CPU executor for GPU work, unsafe stays in `*_unsafe.rs`.
- No private reference, GPU UUID or token (`scripts/ci/no_private_refs.py`,
  `scripts/ci/no_gpu_uuids.py`), and nothing that came from reverse engineering (see below).
- A user-visible change updates `docs/howto/` in the same pull request.

## Where values come from

Every register offset, class set and per-die constant must be verifiable by anyone, from NVIDIA's open
GPU kernel modules, from open captures made with this repository's tools, or from a working boot. Do not
contribute material taken from decompiling, disassembling or otherwise reverse engineering closed
drivers (offsets, symbol names, layouts, notes), and do not contribute leaked code. If your only
evidence for a value is such analysis, say so in the pull request and leave the value out. The full rule
is [`docs/design/V3_PROVENANCE.md`](docs/design/V3_PROVENANCE.md).

## Security

kayfabe treats the guest as hostile. Report a suspected vulnerability privately, as described in
[`SECURITY.md`](SECURITY.md), not in a public issue.

## Licence

kayfabe is dual-licensed `Apache-2.0 OR GPL-2.0-or-later` (see [`LICENSE`](LICENSE) for the exceptions).
Unless you state otherwise, a contribution you submit is licensed the same way, with no additional
terms.
