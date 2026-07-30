# Running the whole suite on real hardware — and the census of what runs where

**Status:** measured 2026-07-30 on a rented RTX 3090 (GA102) box, 38 cores / 73 GB, driver
580.159.04, `HEAD = 5c4cb0d` + this branch. Every number below was produced by
`scripts/run_full_suite.sh` on that box; nothing here is inferred from CI.

## 0. The ruling this file exists to serve

> *"its fine that many tests do not run under github ci … this project cannot rely on gh for
> iterating for free hardware, so gh is purely opportunistic convenience at this point.
> **ensure whole suite can still be run if you have real gpu.**"* — owner, 2026-07-30

**GitHub CI is opportunistic convenience. It is not the definition of green.** The
authoritative run is the one on real hardware, and it is:

```sh
scripts/run_full_suite.sh                 # auto-detects the profile from the box
scripts/run_full_suite.sh --list          # the phase table, run nothing
```

Exit `0` means *everything this box can run, ran*. Anything else means it did not — either a
phase failed, or a phase was skipped for a reason the box's **profile** says it should have
satisfied. See §4 for why the profile, and not the probe alone, is what makes a skip red.

## 1. ★★★ The failure this is written against

**A test that is COUNTED but never PASSES is worse than a missing test**, because it inflates
the number and buys false confidence. Nobody re-checks a green count.

The repo has two live instances, and both are *honest* about it, which is the standard the
runner generalises:

| gated set | on GitHub | why |
|---|---|---|
| the 51 `/dev/kvm` tests | counted, never run | `ubuntu-latest` has no `/dev/kvm` |
| the 10 VBIOS-oracle tests | counted, never run | the runner has no vendored open-kernel-modules trees, and nothing is vendored here to stand in |

Both print a marker on the *skipping* arm — `<FAMILY>-GATE: SKIPPED <test> — <reason>` —
straight to `stderr`, deliberately bypassing libtest's capture because capture swallows
output on the **passing** path, which is the arm that matters. CI then floors the
`RAN + SKIPPED` count so the set cannot silently shrink to nothing.

The runner does not know those families by name. It **derives** them from the markers the run
emits, and on a box whose profile promised the capability a single `SKIPPED` marker is a red
run.

## 2. ★★ The census — where everything runs, and what happens when it can't

### 2.1 The four gate families (all LOUD SKIP, none `#[ignore]`)

| family | requirement | mechanism | count | absent ⇒ |
|---|---|---|---|---|
| `KVM-GATE` | `/dev/kvm` open RW, `KAYFABE_NO_KVM` unset | `kayfabe_linux_raw::require_kvm!` (`crates/kayfabe-linux-raw/src/kvm_gate.rs`) | **51** | LOUD SKIP, both arms printed, CI floor 35 |
| `SANDBOX-GATE` | mount / user namespaces (measured by a throwaway `unshare`, **not** by `geteuid`) | `require_sandbox!` / `require_user_namespace!` (`crates/kayfabe-linux-raw/src/sandbox_unsafe.rs`) | **10** | LOUD SKIP, CI floor 10 |
| `VBIOS-ORACLE-GATE` | the vendored ogkm 580 **and** 610 trees + a C compiler | `require_oracle!` (`tests/tests/vbios_real_parser_oracle.rs`), fed by `tests/build.rs` | **10** | LOUD SKIP, CI floor 10 |
| `SKIPPED (slow)` | `KAYFABE_SLOW=1` | `kayfabe_tests::skip_slow!` (`tests/src/lib.rs`) | **5** | LOUD SKIP, no CI floor — the nightly `slow` job runs them instead |

There is **no** `#[ignore]` attribute anywhere in the tree. A run reports exactly one
`ignored`: a ```` ```ignore ```` fenced block in `tests/src/teardown.rs`'s module docs, which
rustdoc counts as an ignored doc-test. `gate-census` pins that allowance at 1.

**No test anywhere gates on a real GPU.** `/dev/nvidia*` appears in production code and in
one hand-run diagnostic binary; the isolate suite drives `RmMode::Loopback`, and
`crates/kayfabe-isolate-host/tests/real_isolate.rs` says so in its own header — the real-hardware
concurrency measurement is *deliberately* a program rather than a test, "because it needs a
GPU and a wall clock, and a timing assertion in CI is a flake with a justification".

### 2.2 HARD FAIL — resources whose absence is a red build, on purpose

| resource | consumer | polarity |
|---|---|---|
| `x86_64-unknown-linux-musl` std | `crates/kayfabe-isolate-host/build.rs` (the isolate image is `include_bytes!`d as a static musl binary) | every cargo command in the workspace fails. Only escape: `KAYFABE_ISOLATE_IMAGE_STUB=1`, reviewed for the aarch64 *check* job alone |
| a C compiler | `tests/build.rs`, **only when an ogkm tree is present** | tree present + compile fails ⇒ `panic!`. That is not "a machine without the oracle", it is an oracle that has rotted, and degrading it to a skip is how a gate stops being able to fire |
| `traces/cap1_coldboot_hermetic.rec` (13 MB, committed) | `kayfabe-crec`'s two suites | `panic!` naming the path and `KAYFABE_C_TRACE_CAP1` |
| `scripts/ci_gates.sh`, `.github/workflows/ci.yml`, `qemu/hw/misc/nvkvm/kayfabe_shim.h` | `gate_runner_floor.rs`, `gate_scope.rs`, `wire_mirror.rs` | `panic!` — these tests assert *about* those files |
| Linux | `crates/kayfabe-linux-raw` | `compile_error!` |

### 2.3 ★★★ SILENT PASSES

**Zero** — measured by the census sweep of 2026-07-30 over `HEAD = 5c4cb0d`, re-checked by the
`gate-census` phase of the run recorded in §7 (RTX 3090 box, `KAYFABE_SLOW=1`, KVM and both
ogkm trees present), and standing evidence thereafter: `gate-census` fails on any `SKIPPED`
marker, and `tests/tests/full_suite_ledger.rs` fails if the instrument that emits them is
weakened. That zero is a real property of the tree rather than luck: every
environment-conditional early return in a `#[test]` writes a marker to stderr first.

Two defects of the *instrument* were found by this census and fixed in the same commit as
this file. Neither was a silent pass; both were a marker that lied:

1. `the_two_vendored_ogkm_tags_agree_on_our_image` tested its precondition **after**
   `require_oracle!`. It needs *two* trees; `require_oracle!` is satisfied by *one*. So on a
   one-tree box the test printed `VBIOS-ORACLE-GATE: RAN`, returned, and asserted nothing —
   and CI's reached-count recorded it as having executed. The precondition now comes first
   and emits exactly one countable marker whichever way it goes.
2. `a_child_without_the_sandbox_inherits_every_capability_its_parent_holds` announced its
   vacuous-bite case with `eprintln!`. libtest's capture swallows that on the **passing**
   arm — the only arm it could ever appear on. It writes straight to stderr now.

### 2.4 ★★★ What had never run ANYWHERE

| thing | why it ran nowhere | now |
|---|---|---|
| `crates/kayfabe-abi/gen` — **22 unit tests** | its `Cargo.toml` carries its own `[workspace]` table, which detaches it from the root members list. No `--workspace` command reaches it and no CI job names it. These are the C-type scanner and declaration parser that **produce** the committed Axis-A wire tables | `abi-gen-tests` phase — 22 passed |
| `fuzz/` — 114 corpus files + 1 committed `crash-…` artifact | the `nightly-fuzz` job only *compiles* the harness ("running it is manual"), and nothing in the suite reads the corpus | `fuzz-run` + `fuzz-corpus-replay` phases |
| `kayfabe-rm-ladder` | the only code path in the tree that issues a real NVIDIA RM ioctl. Invoked by no test and no script — it ran when a human remembered to type it | `rm-ladder` + `rm-ladder-concurrency` phases |
| `crates/kayfabe-abi/examples/synth_vbios`, `crates/kayfabe-crec/examples/cap1_report` | compiled by `cargo test`, run by nothing | `examples` phase |
| `qemu/hw/misc/nvkvm/nvkvm.c` (the C half of the L2 adapter) | compiled by nothing in this repository and by no CI job | `qom-shim` phase (needs a hypervisor source tree) |

`workspace-census` is what makes this durable: it **discovers** every `[workspace]` root in
the tree and fails on one that no phase covers, so the next detached sub-project cannot
repeat the trick.

## 3. The phases

`scripts/run_full_suite.sh --list` is the list of record. In summary:

* **`ci-stable`** — the entire GitHub `stable` job, via `scripts/ci_gates.sh --all`, which
  *extracts* the steps from `ci.yml` rather than copying them. Build, the `KAYFABE_NO_KVM=1`
  OS-free test configuration, the three reached-count floors, clippy `-D warnings`, fmt, and
  the twelve boundary/vocabulary/unsafe-surface/ABI-quarantine greps.
* **`test-hardware`** — the authoritative suite run: `KAYFABE_SLOW=1 cargo test --workspace
  --no-fail-fast` with `/dev/kvm` present, namespaces permitted and both ogkm trees present.
  This is the run CI can never do.
* **`target-census` / `gate-census` / `workspace-census`** — the three derived censuses over
  that run (§2, and `scripts/run_full_suite.sh`'s own comments for the arguments).
* **`aarch64`**, **`fuzz-build`**, **`tsan`**, **`mutants`** — the other CI jobs.
* **`abi-gen-tests`**, **`fuzz-run`**, **`fuzz-corpus-replay`**, **`rm-ladder`**,
  **`rm-ladder-concurrency`**, **`sandbox-probe`**, **`examples`**, **`qom-shim`** — the
  things no job runs.

## 3.1 ★★★ A red caused by the BOX must not look like a red caused by the CODE

Three of these landed in one evening, each costing a wasted verification cycle:

| what was reported | what it actually was |
|---|---|
| `KVM-gate: ran=0 skipped=0 total=0` — *the gated tests vanished* | a sibling process writing the same `/tmp/kayfabe-test.log`. The run's own output had just printed 51 `KVM-GATE: RAN` |
| "failed, exit 1" from a background verification | the box had filled to **zero bytes** mid-run |
| a seam gate failing on a crate nobody recognised | a shared tree, and another agent's untracked crate. Correct gate, contaminated input |

All three are the same shape, and it is the worse half of this whole subject: a *confident
wrong answer*. A missing check is at least visibly missing. Three defences, all in this
change:

1. **The test log path is `${KAYFABE_TEST_LOG:-/tmp/kayfabe-test.log}`**, and `ci_gates.sh`
   exports a per-invocation `mktemp` path. GitHub's behaviour is byte-identical (its default
   is the old path); concurrent local runs can no longer see each other.
2. **The producer writes a completion sentinel** (`KAYFABE-TEST-LOG-COMPLETE rc=…`) and each
   of the three reached-count steps **refuses, with exit status 2**, over a log that is
   missing, empty, or has no sentinel. *"The producer ran and genuinely found zero"* and
   *"I could not read my input"* are now different answers.
3. **`run_full_suite.sh` measures the box before it measures the tree** — free space on the
   repo filesystem and on `$TMPDIR`, and `git status --porcelain`. It **refuses to start**
   below `DISK_REFUSE_MB` (exit **4**, its own status), warns below `DISK_WARN_MB`, and
   prints the box state again in the ledger — on green runs too, so it is a line people can
   read rather than an alarm they only meet in a crisis. A dirty tree is not an error, but
   it *is* reported, because the boundary / vocabulary / unsafe-surface / ABI-quarantine
   gates grep the tree **as it is on disk**.

## 4. Why a PROFILE, and why `--allow-skip` is the only escape

A ledger built on probes alone is circular: *"the resource was absent, so skipping was fine"*
is true on every machine and rules nothing out. The **profile** is a claim the operator makes
about the machine, which the ledger then checks:

* `gpu-box` — the authoritative configuration; every requirement is expected.
* `ci` — what a GitHub runner can honestly satisfy (`musl`, `pyyaml`).

A skip whose unmet requirement is in the profile's set is **red**. `--allow-skip <phase>` is
the only way to get a clean exit with a phase unrun; it must name the phase, and the ledger
prints it as `ACKNOWLEDGED`. There is deliberately no blanket flag.

## 5. ★★ The recipe — standing a box up from nothing

Vast.ai hosts are **ephemeral** (standing owner directive): nothing persistent, no snapshots,
and a wedged instance gets destroyed rather than nursed. The recipe is what survives.

```sh
# 0. the box: KVM + a GA10x GPU. Filter offers on reliability2 >= 0.99 — "verified" does
#    not discriminate (every KVM offer reports it).

# 1. toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
export PATH=$HOME/.cargo/bin:$PATH
rustup target add x86_64-unknown-linux-musl        # ★ NOT optional: the isolate image is
                                                   #   EMBEDDED as a static musl binary, so
                                                   #   without it every cargo command fails
                                                   #   inside a build script
rustup target add aarch64-unknown-linux-gnu        # the arch-portability check job
rustup toolchain install nightly --component rust-src --profile minimal
rustup target add --toolchain nightly x86_64-unknown-linux-musl   # the nested isolate build
rustup target add --toolchain nightly x86_64-unknown-linux-gnu    # -Zbuild-std for TSan
cargo install cargo-fuzz
cargo install --locked cargo-mutants

# 2. host packages
apt-get update
apt-get install -y python3-yaml \
  ninja-build meson pkg-config libglib2.0-dev python3-venv flex bison libpixman-1-dev

# 3. the vendored NVIDIA open kernel modules — NOT vendored in this repo (they are NVIDIA's,
#    MIT/GPL-2.0). Two tags, because ten oracle tests compare them against each other.
mkdir -p /workspace/nvidia-gpu-passthrough/research_clones
git clone --depth 1 -b 580.159.04 https://github.com/NVIDIA/open-gpu-kernel-modules \
  /workspace/nvidia-gpu-passthrough/research_clones/ogkm-580.159.04
git clone --depth 1 -b 610.43.02 https://github.com/NVIDIA/open-gpu-kernel-modules \
  /workspace/nvidia-gpu-passthrough/research_clones/ogkm
#    …or put them anywhere and set KAYFABE_OGKM_580 / KAYFABE_OGKM_610.

# 4. a hypervisor source tree for the QOM shim (the C half of the L2 adapter). The shim's
#    compile-time floor is 9.2; 9.2.0 and 10.2.4 are the releases it has been BUILT against.
git clone --depth 1 -b v10.0.0 https://gitlab.com/qemu-project/qemu /root/qemu-src
export KAYFABE_QEMU_SRC=/root/qemu-src

# 5. the repo, and then
scripts/run_full_suite.sh
```

★ **If you `rsync` or `cp -a` the tree, `touch` it afterwards.** Preserved mtimes make cargo
serve stale rlibs and report symbols missing that are plainly present; that has fired
repeatedly. `find <tree> -path <tree>/target -prune -o -exec touch {} +`.

★ **`pgrep -x qemu-system-x86`, never `qemu-system-x86_64`** — `/proc/PID/comm` truncates at
15 characters, so the long form can never match and any "nothing is running" check built on
it passes vacuously. `pkill -9 -x`, never `-f`.

## 6. What this does NOT make runnable, named

* **A guest VM.** Nothing in this suite boots one. The stock-driver milestone is measured by
  hand against a QEMU built from the `qom-shim` phase's output; that is a separate procedure
  and it is not automated here.
* **`mutants`.** The campaign is hours, and it is a weekly cron in CI for that reason. The
  runner treats not running it as a skip that must be acknowledged by name
  (`--allow-skip mutants`), so it can never be quietly absent.
* **The completion plane.** Unchanged and unrelated to this runner, but worth repeating
  because it bounds what a green run means: the C artifact *forges* completions, so the
  C↔Rust differential says nothing about that plane (`docs/design/c_rust_trace_differential.md`).
