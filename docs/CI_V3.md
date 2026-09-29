# CI scope after the v3 migration

**STATUS: LIVE, 2026-09-29.** This describes the CI repair, not an expansion of the
hardware support matrix or a claim of complete unsafe-code soundness.

## What remains mandatory

`cargo test --workspace` still runs **all** retained and v3 workspace tests. No
failing test is allowlisted. The OS-free job forces KVM absent, with reached-count
guards; the scheduled/manual slow job exercises the longer tests. The aarch64
cross-check remains. Hardware acceptance is separate: exact-revision GPU gates,
QEMU build, fresh guest, and the 30-arm thin suite (see `CLAUDE.md`).

The repair keeps the legacy tests' safety assertions, updates paths for the
grader's move, adds v3 ABI reliance metadata to the capture guard, and teaches
the frozen doorbell model's generated-class parser to explicitly exclude optical
flow (which that old model never implemented). Unknown class kinds still fail;
the current `kf-chip` coverage is retained. An actual error-code collision in
`kf-host` was fixed. The grader's `--cuda-store-probe` feature check now names
its real `cuda-window` feature instead of a nonexistent local feature.

V3's QEMU wire header now has a compiled C/Rust differential test for size,
alignment, member offsets and ABI version. Foreign CUDA API layouts stay in the
named raw adapter; the CUDA walk wire structs retain their existing differential
proof. Archived QEMU headers are not live ABI declarations.

## Architecture gates

- Every `kf-*` member is checked for old-tree dependencies, including renamed,
  workspace-inherited, build, development and target-specific dependencies.
- Pure v3 algorithm/vocabulary crates have an explicit dependency boundary:
  they cannot import the host/OS adapters or an arbitrary external crate.
  Adapter comments may explain OS mechanisms; legacy comment-vocabulary guards
  are not misapplied to the new architecture.
- V3 unsafe quarantines are the already documented `kf-linux-raw`, `kf-cuda`
  and `kf-qemu`. The retained grader keeps its legacy raw-OS and CUDA adapters.
  Forbid inheritance, raw-pointer naming and relaxation-count gates cover both.
  Counts record the existing surfaces (127/43 and 73/73/36 respectively), not a
  new permission to put unsafe code in other crates.
- `kayfabe-doorbell` is a frozen prototype, not the v3 trap plane. The retained
  `kayfabe-chips` consumes its generated class table. That one legacy dependency
  remains; new consumers are rejected. Its pre-existing naming/lint exceptions
  are bound to SHA-256 hashes of **every Rust source and its manifest**. Added,
  removed or changed files fail that guard. Its tests still execute. This
  exception does not authorize new unsafe surfaces in the prototype.

## Visible migration debt, not warning-free code

The previously red workflow had hundreds of existing lint diagnostics and stale
numeric prose ceilings after the v3 copy/archive. CI now separates failed tests,
compiler errors and denied Clippy errors (always fatal) from existing warning
and prose debt. `scripts/ci/{clippy,claims}-debt.json` records diagnostic identity
by path, category and normalized source text. **Any new site fails**, even if
another old site was removed. An old item's line may move without buying a new
exception. The claim key covers the full paragraph, not just its display excerpt.

This is a deliberate change from `-D warnings` for the whole historical
workspace; it does **not** claim the recorded warnings were fixed or that old
unsupported prose became evidence. Initial Clippy diagnostics were inventoried
with Rust 1.98.0. Style, documentation and dead-code debt is still visible in the
JSON inventory. Denied undocumented unsafe sites were fixed, overlapping match
ranges made disjoint, and the unexpected feature condition corrected rather than
treated as proof that the warning was harmless. The existing documentation-only
unversioned-citation ceiling was reconciled to 172; source citations still fail
outside the hash-frozen prototype.

`--record` is a deliberate maintenance operation, **never run by CI**. Do not
refresh an inventory merely to silence a regression. Review the added keys and
explain them in a commit. Upgrading the compiler may require reviewing new lints.
The Python self-tests check that removed debt cannot buy unrelated new debt,
truncated/failed Clippy output cannot pass, and dependency aliases cannot bypass
the v3 boundary. The QEMU process-identity shell regression test also runs in CI.

Run `bash scripts/ci_gates.sh` for fast guards, or `--all` for the build/test/lint
steps too. The runner extracts the actual workflow and enforces minimum step
counts, so a shortened or malformed job cannot report a vacuous green.
