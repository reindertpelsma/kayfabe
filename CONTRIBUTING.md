# Contributing to kayfabe

**STATUS: LIVE, 2026-10-30.** kayfabe is a research project with one maintainer. Contributions are
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

The package list, the QEMU build and the extra targets are in
[`docs/howto/building.md`](docs/howto/building.md). Tests that need a reference tree (an
NVIDIA open-gpu-kernel-modules checkout, say) say `SKIP` on stderr when it is missing; that is not a
pass. To run them, fetch the tree you need:
`git submodule update --init --depth 1 third_party/ogkm`.

Anything that runs on a real GPU (the hardware gates, the guest suites) needs an NVIDIA GPU and
the open NVIDIA host driver; see [`docs/howto/requirements.md`](docs/howto/requirements.md). You do not
need to run those for a documentation or GPU-free change, and CI does not either, except on the batch
pull request into `master`.

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
