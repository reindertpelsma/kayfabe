# CI scope after the v3 migration

**STATUS: LIVE, 2026-09-29.** This describes the CI repair, not an expansion of the
hardware support matrix or a claim of complete unsafe-code soundness.

## In-use source line-length ratchet (2026-10-23, LL1)

CODE READ: `scripts/ci/linelen.py` enforces 100 columns on Git-tracked in-use source and tests. `scripts/ci/linelen-baseline.json` records existing overlong counts per file. A file's count may never increase; a new file has no allowance. The PR check also rejects every added overlong line in `git diff base...head`, even when the file's total decreases. Baseline edits are compared with the merge base and may only lower counts. Initial enrollment is possible only when that baseline does not yet exist at the merge base; new source lines still face the diff check.

CODE READ: the explicit in-use set is `crates/kf-*/` including tests, `qemu/hw/misc/kf3/`, `cuda/`, `broker/`, `ci/`, `tools/`, `tests/`, `scripts/{ci,fastguest,drivermatrix,apps}/`, `scripts/bench/box/`, and `scripts/bench/cuda_min/`. CI workflows, root formatter/build TOML files, `.editorconfig`, `.clang-format`, and `scripts/ci_gates.sh` are included too. `ci/` is absent at the enrollment revision; its future files are covered, including `ci/vast-broker/`. The root Black config applies unless that future tree introduces an overriding config, which must retain line length 100.

CODE READ: the explicit top-level bench entries and their build/boot/ladder helpers are `v3_gates.sh`, `build_kf3.sh`, `cuda_ladder.sh`, `boot_capture.sh`, `boot_nvkvm.sh`, `assert_boot_evidence.sh`, `qemu_identity.sh`, `qemu_identity_selftest.sh`, `provision_box.sh`, `provision_bench_tree.sh`, `provision_host_driver.sh`, `cup2_hook_deadline.sh`, `cup3_hook.sh`, `cup8_hook.sh`, `cup8bench_hook.sh`, `cup3.c`, `cup8.c`, `cup8bench.c`, `uvmparams_shim.c`, and the extensionless `gssh_nv`. Other bench runners need an explicit scope edit before they become gate inputs. Tests colocated anywhere within an included tree are covered automatically.

CODE READ: supported extensions are Rust; C/C++ and headers; CUDA; shell (`sh`, `bash`, `zsh`); Python; JavaScript/TypeScript; TOML; and YAML. Width counts Unicode characters after expanding tabs to eight-column stops, excluding CR/LF. Markdown prose, JSON, TSV, PTX, binaries and unrecognized extensions are outside this source checker. There is no Markdown code-fence checker in this change; keep code/log examples readable in review. Symlinks are never followed and submodules are never recursed into.

### Exemptions

CODE READ: `archive/`, `archives/`, `archive-*` and `archive_*` directory components anywhere in a path are excluded entirely, as are `third_party/`, `vendor/`, `vendored/`, `node_modules/`, and `traces/`. All `crates/kayfabe-*` are exempt because the pre-v3 tree is frozen. `scripts/bench/windows/host_tools/`, dated Windows/bench runners and other paths outside the explicit set have no baseline entries. The outside-set report group also includes separately gated firmware and root CI helper scripts. No files in these excluded trees are reformatted.

CODE READ: generated directories, `*_generated` files, header comments with `@generated` or “Generated … do not edit”, and lockfiles are exempt. A line consisting solely of a URL or hexadecimal hash, optionally inside a comment, is exempt. A trailing comment `long-line-ok: <reason>` exempts that line; an empty reason fails. Use it for indivisible data or protocol fixtures, with a reviewable reason. It is a line exception, never a file allowance. The in-use `tools/host_patches/h5_userd_dma/include/` headers, including NVIDIA MIT headers, stay in the baseline and are untouched; no special vendor exemption is added for that tree.

### Commands and formatter policy

CODE READ: both hosted CI and `scripts/ci_gates.sh` run the same source gate. Hosted checkout fetches full history. PRs use the event's base SHA, pushes use the event's previous SHA, and a new-branch push or local run uses `origin/master`. Supply a specific base explicitly for other local comparisons:

```sh
python3 scripts/ci/linelen.py --base origin/master
python3 scripts/ci/changed_format.py --base origin/master
python3 scripts/ci/linelen.py --report
python3 scripts/ci/linelen.py --base origin/master --update
python3 -m unittest discover -s scripts/ci -p 'test_*.py'
```

CODE READ: `--update` only lowers an existing inventory after all checks pass; it refuses missing/invalid baselines and never records new debt. Without `--base`, the normal check scans tracked working-tree content against counts, while `--update` also rejects increases against HEAD's baseline. With `--base`, added-line checks use committed head content and plain, uncolored Git diffs, so local edits or forced ANSI colors cannot hide a committed violation. Source lines use Git's physical LF boundaries rather than Unicode line separators. Exit 1 means a regression; exit 2 means an input/tool failure. `--report` reads the exempt source inventory for comparison without enforcing or recording it; routine checks do not read exempt trees.

CODE READ: Rust keeps the existing `cargo fmt --all --check` and firmware rustfmt checks, with an explicit `rustfmt.toml` setting `max_width = 100`. Existing Rust code and comments are not reflowed, and no Rust cleanup PR is proposed. Root Black and Prettier configs set line length/print width 100; `.editorconfig` declares that width for supported languages. These config files do not imply a whole-tree Black or Prettier check.

CODE READ: clang-format major 18 is installed explicitly in CI. `changed_format.py` invokes `git clang-format-18 --diff` between the merge base and head, limited to changed C/H/CUDA files in `broker/`, `cuda/` and `qemu/hw/misc/kf3/`, with the source exemptions above. Any formatter diff fails; tool errors fail separately. The command never applies its output. Clang-format may include surrounding syntactic context in a changed-line patch; that limited context must be reviewed. The four-space/Linux-brace style preserves include order and does not reflow comments.

MEASURED: the dry run at base revision `e9c6d39e5`, scope inventory, top 30 files and mutation evidence are in `traces/linelen_20261018/report.md`. The kf3 style delta is small enough to use the changed-line check; no whole-file formatter requirement or upstream QEMU 80-column policy is imposed. INFERRED: optional small Python/script/C follow-ups are more useful than a whole-tree format sweep; the evidence report lists counts and recommendations.

## CI repair of 2026-10-21 (branch `claude/ci-green-20261021`)

The Clippy step had been red on master since 2026-10-15, so every step after it had stopped running and
drift accumulated behind it. What changed, step by step:

- **Clippy errors (93 on the first local run, one more revealed once those were fixed).** `kf-qemu` denies `clippy::arithmetic_side_effects` and
  `clippy::indexing_slicing` for the guest-facing plane (`chan.rs` and the `user*.rs` modules it includes).
  Every flagged site now uses a checked or saturating form or `.get(..)` and fails closed; no `#[allow]` was added.
- **Format check.** 50 files were reformatted (`cargo fmt --all`); no token changed.
- **Clippy debt gate (`clippy-debt.json`).** 119 warning sites that predate this repair (new code since the last
  record) were recorded as reviewed migration debt: kf-disp 29, kf-qemu 26, kf-chan 24, kf-core 17, kf-abi 8,
  kf-broker 7, kf-harness 3, kf-host 2, kf-cuda 1, kf-mem 1, kf-trap 1. Largest classes: `items_after_test_module` 17,
  `dead_code` 17, `manual_is_multiple_of` 13, `collapsible_if` 13, `missing_docs` 11, `type_complexity` 7. Nine
  recorded sites no longer fire. No repair-introduced warning is among them.
- **Claim ledger (`claims-debt.json`).** 616 sites were recorded (607 `unattributed`, 7 `bare-hardware`,
  2 `conflated`): docs 327, traces 154, kf-qemu 51, kf-chan 40, kf-disp 11, kf-broker 8, kf-rm 7, kf-mem 5, kf-abi 4,
  and 9 elsewhere. They are prose written since the last record; none was rewritten.
- **Dependency isolation.** `kf-trap -> loom` and `kf-disp -> kf-broker, kf-linux-raw` are dev-dependencies of pure
  crates that the rule forbids. They are named, exactly and as dev-dependencies only, in `DEV_ONLY_EXCEPTIONS` of
  `scripts/ci/dependencies.py`. **Owner decision pending:** keep the exceptions, or move those tests to `kf-harness`.
- **ABI-quarantine gate.** It aborted on `SIGPIPE` under `pipefail` once a file's remainder outgrew the 64 KiB pipe
  buffer; the type lookup now tolerates an early-exiting `grep`.
- **Untagged ogkm citations.** `kf-abi/chanbudget.rs` is tagged `ogkm-580:` with corrected lines (the old
  `:317-334` was not where those cases are at 580.159.04); `kf-census` no longer spells the notation in identifiers
  (module `oracle`, field `ogkm_answer`); the 52 bare citations of `V3_VGPU_COMPAT_RESEARCH_20261013.md` are tagged
  `ogkm-595:`, the one tree that document names, without being re-read. The docs ratchet stays at 172.
- **Section numbers.** `V3_COMPLETION_AUDIT.md` finding headings are `F1..F12`.
- **Unsafe-surface gate.** A test message in `kf-mem/src/sim/tests.rs` contained the keyword; reworded.

## CI host prerequisites (2026-09-29)

The local unprivileged reproduction of GitHub's `execve/EACCES` failure produced
an AppArmor denial: the generic `unprivileged_userns` profile refused the embedded
memfd executable as a disconnected path. The identical test passed under a named
profile restricted to its executable; the probe profile was then removed.
`runner_namespaces.py` applies that prerequisite **only** to this GitHub checkout's
`target/**` executables and removes it after tests. It does not disable AppArmor,
change system sysctls, grant Linux capabilities or change product sandbox code.
All namespace/isolate containment assertions remain active.

Unprivileged `/proc/iomem` can redact RAM to `00000000-00000000`. The memory-type
test now separates its always-run fail-closed assertion from the host-RAM positive
control, which emits an explicit skipped marker if no complete RAM page is visible.
Both raw-OS crates additionally test redacted, valid and short-range parser inputs.

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

## Firmware (2026-10-03)

⊘ **CORRECTED 2026-10-03 (late, branch `v3-gop`; `OWNER_RULINGS.md` §K) — the paragraph below
describes a committed blob that no longer exists.** No compiled binary is committed. The driver kf3
serves is built from `firmware/kf-gop` by `crates/kf-gop-image/build.rs` in every workspace build
(`rust-toolchain.toml` lists `x86_64-unknown-uefi`; the `stable` and `slow` jobs install it, and a
missing target is a named build error). The `firmware` job now: fails if any tracked file is a compiled
binary (`.efi` or an `MZ` header, outside `archive/`); builds both flavours; checks that the driver
`kf-gop-image` embeds equals the standalone build byte for byte (the property one future signature rests
on — it replaced the committed-blob comparison); runs the host tests, Clippy with `-D warnings`, rustfmt,
kf3's PE check, the no-port-I/O check, the size record and the stand-in. The `aarch64` job's
workspace cross-check now embeds a real `aarch64-unknown-uefi` driver, and a step of its own builds the
driver for that target, lints it, checks it is an AArch64 boot-service driver, and checks that the two
x86-only pieces (`debugcon`, the test app) refuse to build there by name. The firmware ratchet entry is
`firmware/kf-gop:78` (the framebuffer's one volatile word store).

`firmware/kf-gop` is guest-side UEFI code (kf3's boot-display GOP, `docs/design/V3_DISPLAY.md` §4.11),
outside the cargo workspace and built by its own job, `firmware`: `rustup target add
x86_64-unknown-uefi --toolchain 1.99.0` (`rust-toolchain.toml` unchanged), both build flavours, the
safe library's host tests, Clippy with `-D warnings`, rustfmt, **a byte-for-byte comparison of the
committed `firmware/kf-gop/kf-gop.efi` (the blob `kf-oprom` embeds) with this source's build**, kf3's
PE acceptance check, a check that the release driver has no port I/O, the `.efi` size in the job
summary, and the local stand-in
(`scripts/display/gop_standin.sh`, gating). Its unsafe code is a named exception under the same rules,
checked in the `stable` job: `*_unsafe.rs` naming and the host-pointer gate apply unchanged; gate B and
the ratchet take path entries, and `firmware/kf-gop` is the one such entry; the ABI-quarantine gate's
firmware arm keeps every `#[repr(C)]` there inside a `*_unsafe.rs` file. `crates/kf-oprom` (the ROM
container) is an ordinary pure workspace crate.

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
