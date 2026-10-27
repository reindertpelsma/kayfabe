# CI scope after the v3 migration

**STATUS: LIVE, 2026-09-29.** This describes the CI repair, not an expansion of the
hardware support matrix or a claim of complete unsafe-code soundness.

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

## Documentation freshness policy (2026-10-22)

CODE READ: [`doc_freshness.py`](../scripts/ci/doc_freshness.py) extends C1's
[`doc_status_check.py`](../scripts/ci/doc_status_check.py), introduced in PR #87.
It checks active Markdown under `docs/`, root `ARCHITECTURE.md` and root Markdown
README files. Each opens with a dated STATUS paragraph after an optional title:
LIVE, SUPERSEDED, ANSWERED, DESIGN-ONLY or RESEARCH. SUPERSEDED metadata links an
existing successor file in the same paragraph. Relative Markdown links, images,
reference links and HTML destinations must resolve inside the checkout. Comments,
fenced examples and inline code examples are excluded from link checking.
Markdown fragments resolve against common heading IDs (including duplicates and
setext headings) or explicit HTML anchors. External URLs and fragment semantics
for non-Markdown assets are outside this check.

CODE READ: a LIVE status older than `--max-age-days` (default 60 days) becomes
STALE-SUSPECT when it also has a missing literal reference. The checker searches
tracked source with `git grep`, reads crate package names from manifests, checks
file paths, and enumerates local/remote-tracking branch refs. It recognizes KF
family environment names, generic environment assignments and named environment
variables, and long command-line flags. Source grep excludes documentation,
archives, traces, third-party files, target directories and checker fixtures.
Short source filenames may resolve by suffix within active source; explicit root
paths must resolve at their stated location. Absolute filenames in code examples
describe host paths and are omitted from repository-reference diagnostics and
reports. Markdown destinations beginning with `/` retain repository-root link
semantics and must resolve. Computed names, external tools and
intentionally historical references need human review. A finding is a review
signal; it does not establish that a document is semantically obsolete.

CODE READ: `scripts/ci/doc-freshness-debt.json` records the initial legacy findings
by path, kind, diagnostic text and SHA-256 of the entire document. Each stale
reference has its own identity, so restoring one does not create new debt for
another. An edited document loses
its exemption and must pass all checks; removing one finding cannot buy a new
finding elsewhere. An unchanged document also fails if a newly missing source or
link target introduces a finding. The age calculation uses the current date;
`--today YYYY-MM-DD` makes a review snapshot reproducible. The explicit `--record`
operation is for reviewed migration debt and is never run in CI. Do not refresh
this baseline to conceal new or edited-document failures. The claims and Clippy
debt inventories are separate and remain unchanged by this workflow.

CODE READ: [the documentation workflow](../.github/workflows/doc-freshness.yml)
runs on documentation PRs, weekly and on manual dispatch. It publishes the full
per-file report as `DOC_FRESHNESS.md` in the job summary and a downloadable
artifact, including status, date, age, missing literal references, successor and
proposed owner action. Non-Markdown files are inventoried as ASSET; archive files
are inventoried as ARCHIVED. Neither receives a fabricated prose status. Report
recommendations are inferences, never automatic document edits or archive moves.

When an entire document no longer applies, move its contents to `archive/` and
leave a one-line SUPERSEDED pointer at the original path. Link the preserved
archive document, or the authoritative replacement when one exists. This keeps
incoming file links working. Verify the pointer and relative paths during the
move; freeze the archive after that transaction. Both `archive/` and the existing
`docs/archive/` are read-only afterward and excluded from active-document checks.
Partial corrections stay above the corrected text in the active parent document.
An active supersession notice should name the original pointer stub rather than
require a later edit to frozen archive text. Whole-document moves require the
owner's classification decision; the checker does not authorize them.

AGENTS-level rule. The owner approved the gate on 2026-10-27 ("good classification of old docs is
important as agents otherwise use it when it has long been superseded"); AGENTS.md carries the short
form (Doc hygiene). The published manual (`docs/howto/`, `docs/FAQ.md`) is inventoried as MANUAL and is
not dated, because the site versions it per release tag. The longer proposal, kept for the archive-and-
pointer policy, which is not yet adopted:

> Every active Markdown document opens with a dated allowed STATUS. Review
> source references before reaffirming LIVE. If the whole document no longer
> applies, move it to archive/ and leave a dated SUPERSEDED link at its old path;
> preserve history and incoming links. Archive contents are read-only after the
> move and excluded from active checks. Changed documents must clear the
> freshness ratchet; never re-record debt to bypass it.
