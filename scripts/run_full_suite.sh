#!/usr/bin/env bash
# ★★★ THE ONE WAY TO RUN EVERYTHING — and the ledger that says what it could not run.
#
# ## Why this exists (owner ruling, 2026-07-30)
#
# > *"its fine that many tests do not run under github ci … this project cannot rely on gh
# > for iterating for free hardware, so gh is purely opportunistic convenience at this
# > point. **ensure whole suite can still be run if you have real gpu.**"*
#
# GitHub CI is opportunistic convenience. It is NOT the definition of green. The
# authoritative run is on real hardware, and there has to be one documented command that
# performs it — including the six CI jobs, the parts CI structurally cannot do (KVM, user
# namespaces, the vendored NVIDIA trees, a real GPU), and an honest statement of what it
# skipped.
#
# ## ★★★ The failure this exists to prevent, measured twice in this repo
#
# **A test that is COUNTED but never PASSES is worse than a missing test.** It inflates the
# number and buys false confidence. Two live instances:
#
#   * the ~51 KVM-gated tests — CI counts them via a reached-count floor and never runs one;
#   * the VBIOS-oracle and GMMU-oracle tests — they need the vendored open-kernel-modules
#     trees, which GitHub runners do not have, so there they are counted and never pass.
#
# All are *honest* about it, because each prints a `…-GATE: SKIPPED <test> — <reason>` line
# on the skipping arm. That discipline is the standard this script generalises: it does not
# know the gate families by name, it **derives** them from the markers the run emits, and on
# a box that is supposed to have the capability a single SKIPPED marker is a red run.
#
# ## ★★ Everything countable here is DERIVED, never hand-listed
#
# A hand-list is a smaller true statement, and the gate quietly weakens the day someone adds
# a target. This repository has been bitten by exactly that three times (a `-maxdepth 1`
# ratchet, a token list, three crate lists). So:
#
#   * the **test-target universe** comes from `cargo metadata`, and every target it names
#     must appear in the run's output (`target-census` phase);
#   * the **gate families** come from `grep -o '[A-Z0-9-]*-GATE: SKIPPED'` over the run log,
#     so a family added tomorrow is counted tonight;
#   * the **skip reasons** come from the markers themselves, because the discipline requires
#     each marker to carry its own reason.
#
# The two things that are literals are literals **on purpose**, for the reason
# `ci_gates.sh` spells out at length: a floor derived from the thing it checks moves
# silently when that thing moves. See `PHASE_FLOOR` and `TARGET_UNIVERSE_FLOOR` below.
#
# ## Usage
#
#   scripts/run_full_suite.sh                  # auto-detect the profile from the box
#   scripts/run_full_suite.sh --profile gpu-box
#   scripts/run_full_suite.sh --profile ci     # what a GitHub runner can honestly do
#   scripts/run_full_suite.sh --allow-skip mutants --allow-skip qom-shim
#   scripts/run_full_suite.sh --only test-hardware,target-census
#   scripts/run_full_suite.sh --list           # phases + requirements, run nothing
#
# Exit status:
#   0  every phase this box is supposed to be able to run, ran, and passed.
#   1  something failed, OR something was skipped for a reason this box's profile says it
#      should have satisfied. **A clean exit means "everything this box can run, ran".**
#   3  the phase list itself is suspect (below `PHASE_FLOOR`) — nothing was run.
#   4  the BOX is unfit (out of disk) — nothing was run, and nothing here is about the code.
#
# ## ★★★ A red whose cause is the ENVIRONMENT must not look like a red whose cause is the
# ## CODE
#
# Three of those landed in one evening, each costing a wasted verification cycle:
#
#   * a reached-count step reported `ran=0 skipped=0 total=0` — which reads as *the gated
#     tests vanished* — because a sibling process was writing the same `/tmp` log;
#   * a background run came back "failed, exit 1" that was **purely disk**: the box had
#     filled to zero bytes mid-run;
#   * a suite run in a shared tree picked up another agent's untracked crate and failed a
#     seam gate on it — the correct gate, over contaminated input.
#
# All three are the same shape, and a ledger is the natural place to separate them. So this
# script measures the box BEFORE it measures the tree, prints the result whether or not it
# is alarming, and refuses outright rather than producing a misattributable red.
#
# `--allow-skip <phase>` is the ONLY way to get a clean exit with a phase unrun, it must
# name the phase explicitly, and the ledger prints it as `ACKNOWLEDGED` — visible in the
# output and visible in whatever invoked it. There is deliberately no blanket flag.

set -uo pipefail
cd "$(dirname "$0")/.."
REPO="$PWD"

# =====================================================================================
# ★★ THE PHASE FLOOR. A literal, checked before anything runs.
# =====================================================================================
# `ci_gates.sh` once printed "ALL GATES CLEAN (0 steps)" and exited 0 because its extractor
# returned nothing. A runner that reports success having run nothing is the purest form of
# the bug this whole script is about, and it hides every other instance. The phase table
# below is static shell, so it cannot be truncated by a parser — but a bad `--only` filter,
# an early `return`, or a future refactor into a generated list all reproduce it through a
# different door. Bump this literal in the same commit that adds or removes a phase.
PHASE_FLOOR=17
# The number of cargo test targets `cargo metadata` must yield before the census is willing
# to believe it. Same argument: an empty universe trivially satisfies "everything ran".
TARGET_UNIVERSE_FLOOR=80
# ★ The `#[ignore]` allowance. The repo's stated position is that NOTHING is `#[ignore]`d —
# an ignored test is invisible in a summary and impossible to count — and the source honours
# it: there is not one `#[ignore]` attribute in the tree. The single ignored entry a run
# reports is a ```ignore fenced block in `tests/src/teardown.rs`'s module docs, which
# rustdoc counts as an ignored doc-test. A literal, not an env knob: the whole point is that
# a second one has to be a diff someone can see.
IGNORED_ALLOWANCE=1
# ★ Every cargo WORKSPACE root in this repository, and therefore every one that a phase
# above must cover. Discovered by `census_workspaces`; pinned here because "each discovered
# workspace is handled" is only a real statement if an undiscovered-yet-added one turns the
# census red rather than silently joining the set.
HANDLED_WORKSPACES=". fuzz crates/kayfabe-abi/gen"
# ★★ Disk. A full `--all` run builds the workspace, an aarch64 check, a release archive, an
# instrumented `-Zbuild-std` std and (with a hypervisor tree) a QEMU — tens of gigabytes of
# target directories. Running out mid-run produces a compiler error that looks like a code
# defect; it was misread as one on 2026-07-30. `DISK_REFUSE_MB` is where this script would
# rather say nothing than say something misattributable; `DISK_WARN_MB` is where it says so
# and continues. Literals, for the same reason every floor here is a literal.
#
# ★★★ 2048 -> 6144, and the 2048 is MEASURED to have been unable to protect anything.
# `[measured 2026-08-09]` a `--all` run started with **6 144 MB free** — comfortably above
# the 2 048 floor, so this script warned and ran — and the box filled to **16 MB** during
# the workspace build. `fuzz-corpus-replay` then died with
# `the nested build of the isolate image failed`, which is a compiler error, which reads as
# a code defect. That is the exact misattribution this pair of numbers exists to prevent,
# reproduced with both numbers in force.
# ⊘ **A refuse floor below what the workload itself consumes cannot refuse anything.** The
# floor is now the largest free-space value at which a run has been OBSERVED to die, so it
# is a measurement rather than an estimate. ⚠ It is a LOWER bound and is honest about that:
# the true requirement is somewhere between 6 144 and `DISK_WARN_MB`, and nothing has
# measured it — raise this floor the next time a run dies above it rather than guessing now.
DISK_REFUSE_MB=6144
DISK_WARN_MB=15360

# =====================================================================================
# Requirements — probe, and the sentence that says how to satisfy it
# =====================================================================================
# Each requirement is `probe_<name>`, returning 0 if satisfied. `fix_<name>` prints the
# remedy. A requirement is never assumed from another: `gpu` does not imply `kvm`.

OGKM_580="${KAYFABE_OGKM_580:-/workspace/nvidia-gpu-passthrough/research_clones/ogkm-580.159.04}"
OGKM_610="${KAYFABE_OGKM_610:-/workspace/nvidia-gpu-passthrough/research_clones/ogkm}"
FWSEC_REL="src/nvidia/src/kernel/gpu/gsp/kernel_gsp_fwsec.c"
VBIOS_REL="src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_vbios_tu102.c"

probe_kvm()   { [ -r /dev/kvm ] && [ -w /dev/kvm ] && [ "${KAYFABE_NO_KVM:-}" != "1" ]; }
fix_kvm()     { echo "a machine with /dev/kvm readable+writable by this user (and KAYFABE_NO_KVM unset)"; }

probe_gpu()   { [ -c /dev/nvidiactl ] && [ -c "/dev/nvidia${KAYFABE_GPU:-0}" ]; }
fix_gpu()     { echo "an NVIDIA GPU with the driver loaded: /dev/nvidiactl + /dev/nvidia${KAYFABE_GPU:-0}"; }

probe_userns() { unshare -Ur true >/dev/null 2>&1; }
fix_userns()   { echo "unprivileged user namespaces (sysctl kernel.unprivileged_userns_clone=1), or run as root"; }

probe_ogkm580() { [ -f "$OGKM_580/$FWSEC_REL" ] && [ -f "$OGKM_580/$VBIOS_REL" ]; }
fix_ogkm580()   { echo "the vendored open-kernel-modules 580.159.04 tree at $OGKM_580 (set KAYFABE_OGKM_580 to relocate)"; }

probe_ogkm610() { [ -f "$OGKM_610/$FWSEC_REL" ] && [ -f "$OGKM_610/$VBIOS_REL" ]; }
fix_ogkm610()   { echo "the vendored open-kernel-modules 610.43.02 tree at $OGKM_610 (set KAYFABE_OGKM_610 to relocate)"; }

probe_musl()  { rustup target list --installed 2>/dev/null | grep -qx x86_64-unknown-linux-musl; }
fix_musl()    { echo "rustup target add x86_64-unknown-linux-musl   (the isolate image is EMBEDDED as a static musl binary)"; }

probe_aarch64() { rustup target list --installed 2>/dev/null | grep -qx aarch64-unknown-linux-gnu; }
fix_aarch64()   { echo "rustup target add aarch64-unknown-linux-gnu"; }

probe_nightly() { rustup toolchain list 2>/dev/null | grep -q '^nightly'; }
fix_nightly()   { echo "rustup toolchain install nightly"; }

probe_nightly_musl() { rustup target list --toolchain nightly --installed 2>/dev/null | grep -qx x86_64-unknown-linux-musl; }
fix_nightly_musl()   { echo "rustup target add --toolchain nightly x86_64-unknown-linux-musl (the nested isolate build keeps RUSTUP_TOOLCHAIN)"; }

probe_rustsrc() { rustup component list --toolchain nightly --installed 2>/dev/null | grep -q '^rust-src'; }
fix_rustsrc()   { echo "rustup component add --toolchain nightly rust-src   (TSan needs -Zbuild-std)"; }

probe_cargofuzz() { command -v cargo-fuzz >/dev/null 2>&1; }
fix_cargofuzz()   { echo "cargo install cargo-fuzz"; }

probe_mutants() { command -v cargo-mutants >/dev/null 2>&1; }
fix_mutants()   { echo "cargo install --locked cargo-mutants   (the campaign is HOURS: --allow-skip mutants to acknowledge)"; }

probe_pyyaml() { python3 -c 'import yaml' >/dev/null 2>&1; }
fix_pyyaml()   { echo "python3 + pyyaml (ci_gates.sh extracts the gate list from ci.yml itself)"; }

probe_qemusrc() { [ -n "${KAYFABE_QEMU_SRC:-}" ] && [ -f "${KAYFABE_QEMU_SRC}/VERSION" ]; }
fix_qemusrc()   { echo "a hypervisor source tree, pointed at by KAYFABE_QEMU_SRC (git clone https://gitlab.com/qemu-project/qemu)"; }

ALL_REQS="kvm gpu userns ogkm580 ogkm610 musl aarch64 nightly nightly_musl rustsrc cargofuzz mutants pyyaml qemusrc"

# =====================================================================================
# Profiles — what a box of this KIND is expected to satisfy
# =====================================================================================
# ★ The profile is what makes a skip red rather than informational. Without it the ledger
# is circular: "the resource was absent, so the skip was fine" is true of every machine and
# tells you nothing. A profile is a claim about the machine that the operator makes and the
# ledger checks.
#
#   gpu-box  — the authoritative configuration. Everything.
#   ci       — what a GitHub runner can honestly satisfy. Named so the CI job can use this
#              same script and still be red for its own reasons rather than for the
#              hardware it structurally lacks.
profile_reqs() {
  case "$1" in
    gpu-box) echo "$ALL_REQS" ;;
    ci)      echo "musl pyyaml" ;;
    none)    echo "" ;;
    *)       echo "★ unknown profile '$1' (gpu-box | ci | none)" >&2; exit 2 ;;
  esac
}

# =====================================================================================
# The phases
# =====================================================================================
# `phase <name> "<requirements>" <<'CMD' … CMD`-ish is tempting; a flat parallel-array table
# keeps the whole list readable in one screen, which is the property that matters for a list
# whose completeness IS the deliverable.
PHASE_NAMES=()
PHASE_REQS=()
PHASE_CMDS=()
PHASE_WHAT=()
phase() {
  PHASE_NAMES+=("$1"); PHASE_REQS+=("$2"); PHASE_WHAT+=("$3"); PHASE_CMDS+=("$4")
}

# --- the whole `stable` CI job, extracted from ci.yml by ci_gates.sh itself ------------
# Build + the OS-free test configuration + the three reached-count floors + clippy + fmt +
# the twelve boundary/vocabulary/unsafe/ABI greps. Delegated rather than duplicated: a
# hand-copied gate list drifts from the real job silently.
phase ci-stable "pyyaml musl" \
  "the entire GitHub 'stable' job — build, KAYFABE_NO_KVM test, reached-count floors, clippy, fmt, 12 boundary gates" \
  './scripts/ci_gates.sh --all'

# --- the run that CI can never do: real KVM, real namespaces, the vendored trees --------
# ★ This is the authoritative suite run. KAYFABE_SLOW=1 opens the measured-slow tests (the
# `slow` CI job), and with /dev/kvm present the ~51 KVM-gated tests actually execute rather
# than being counted. Its log is what `target-census` and `gate-census` read.
phase test-hardware "kvm userns ogkm580 ogkm610 musl" \
  "cargo test --workspace with KVM present, namespaces permitted, the ogkm trees present and KAYFABE_SLOW=1" \
  'KAYFABE_SLOW=1 cargo test --workspace --no-fail-fast'

# --- ★★★ the two derived censuses over that run ----------------------------------------
phase target-census "" \
  "every cargo test target named by cargo metadata appears in the hardware run (derived universe, not a hand-list)" \
  'census_targets'

phase gate-census "" \
  "zero SKIPPED markers in the hardware run — gate families derived from the log, not named here" \
  'census_gates'

# ★★★ The census one level up: `cargo test --workspace` covers ONE workspace, and this
# repository contains three. `fuzz/` and `crates/kayfabe-abi/gen/` each carry their own
# `[workspace]` table, which detaches them from the root members list — so their tests are
# invisible to every `--workspace` command and to every CI job. The generator's 22 unit
# tests had, on 2026-07-30, never run anywhere. This census discovers the workspace roots
# rather than trusting a list, and goes red if one appears that no phase below covers.
phase workspace-census "" \
  "every cargo WORKSPACE in the tree is covered by a phase (a detached sub-workspace is invisible to --workspace)" \
  'census_workspaces'

# --- the remaining CI jobs -------------------------------------------------------------
phase aarch64 "aarch64" \
  "cargo check --workspace for aarch64 (the arch-portability gate; no x86 assumptions)" \
  'KAYFABE_ISOLATE_IMAGE_STUB=1 cargo check --workspace --target aarch64-unknown-linux-gnu'

phase fuzz-build "nightly" \
  "the libfuzzer harness compiles (its own workspace; the only place unsafe deps are allowed)" \
  'cd fuzz && cargo +nightly build'

# ★★ The ABI generator's own workspace. Detached by its own `[workspace]` table, so
# `cargo test --workspace` at the root has never seen it and no CI job names it: its 22
# unit tests (the C type parser and the declaration scanner that PRODUCE the committed
# wire tables) ran nowhere at all until this phase existed.
phase abi-gen-tests "" \
  "★ the Axis-A ABI generator's own 22 unit tests — a detached workspace no --workspace command reaches" \
  'cd crates/kayfabe-abi/gen && cargo test'

# ★★★ Both fuzz phases below are quantified over `cargo fuzz list`, NOT over a name.
#
# They used to name `parse_pushbuffer` and only `parse_pushbuffer`. That was correct
# while it was the only target and silently wrong the moment a second one landed: seven
# more arrived on 2026-08-01 and a hand-named phase would have run one of eight while
# still reporting green. This is `gates_quantified_over_a_list.md`'s exact shape — a
# smaller universe is a smaller true statement — so the universe is derived from the
# `[[bin]]` table `cargo fuzz list` reads, and adding a target to `fuzz/Cargo.toml` is
# the whole of wiring it in.
phase fuzz-run "nightly cargofuzz" \
  "★ EVERY fuzz target in \`cargo fuzz list\` actually EXECUTES (CI only ever compiles them)" \
  'run_fuzz_all'

# ★★ The corpus and the committed crash artifacts are REGRESSION INPUTS that nothing
# replays. They sit in the tree; no test reads them and the nightly job only compiles the
# harnesses, so a re-introduced crash on a known-bad input is invisible. `-runs=0`
# replays each file exactly once and exits.
#
# ⚠ Until 2026-08-01 `fuzz/.gitignore` ignored `corpus` and `artifacts`, so on a fresh
# clone this phase replayed NOTHING and passed. `replay_fuzz_all` therefore counts the
# files it fed and goes red on zero — a replay phase with an empty corpus is the
# counted-but-never-passes failure this script's own header exists to prevent.
phase fuzz-corpus-replay "nightly cargofuzz" \
  "★ replay every committed corpus file AND every committed crash artifact, for every target" \
  'replay_fuzz_all'

phase tsan "nightly rustsrc nightly_musl" \
  "ThreadSanitizer over the four threaded suites (concurrency_stress, rt_shell, l1_verb_seam, l1_mean)" \
  'KAYFABE_SLOW=1 KAYFABE_STRESS_WATCHDOG_SECS=1800 RUSTFLAGS="-Zsanitizer=thread" cargo +nightly test -p kayfabe-tests --test concurrency_stress --test rt_shell --test l1_verb_seam --test l1_mean -Zbuild-std --target x86_64-unknown-linux-gnu -- --test-threads=1'

phase mutants "mutants musl" \
  "the L1-surface mutation campaign (HOURS — acknowledge with --allow-skip mutants if you are not running it)" \
  'run_mutants'

# --- the hardware-only entry points ----------------------------------------------------
# ★★ These are the parts of this project that, before this script existed, RAN NOWHERE.
# `kayfabe-rm-ladder` is the only code path in the tree that issues a real NVIDIA RM ioctl:
# the isolate suite drives `RmMode::Loopback`, and `crates/kayfabe-isolate-host/tests/
# real_isolate.rs` says so in its own header. Nothing invoked this binary from any script or
# job, so its rungs were exercised only when a human remembered to type the command.
phase rm-ladder "gpu" \
  "★ the real RM bring-up ladder R0-R9 against the host driver — the ONLY real-NVIDIA-ioctl path in the tree" \
  'cargo run --quiet --bin kayfabe-rm-ladder -- --gpu ${KAYFABE_GPU:-0} --engines | tee "$LOGDIR/rm-ladder.out"; exit "${PIPESTATUS[0]}"; '

phase rm-ladder-concurrency "gpu" \
  "R12 — the isolate-pool concurrency measurement against the real driver (overlap, not wall clock)" \
  'cargo run --quiet --release --bin kayfabe-rm-ladder -- --gpu ${KAYFABE_GPU:-0} --concurrency | tee "$LOGDIR/rm-ladder-conc.out"; exit "${PIPESTATUS[0]}"'

phase sandbox-probe "" \
  "the isolate sandbox probe binary — what the child can still reach, measured rather than declared" \
  'cargo run --quiet --bin kayfabe-sandbox-probe'

# ★ Examples are COMPILED by `cargo test` and run by nothing. Both of these are diagnostics
# over committed inputs, so running them costs seconds and turns "it still builds" into "it
# still produces an answer".
phase examples "" \
  "the two diagnostic examples actually run (synth_vbios over the generated image, cap1_report over the committed trace)" \
  'cargo run --quiet -p kayfabe-abi --example synth_vbios -- "$LOGDIR/synth_vbios.rom" && ls -l "$LOGDIR/synth_vbios.rom" && cargo run --quiet -p kayfabe-crec --example cap1_report > "$LOGDIR/cap1_report.out" && tail -20 "$LOGDIR/cap1_report.out"'

# --- the C half of the L2 adapter ------------------------------------------------------
# ★ `qemu/hw/misc/nvkvm/nvkvm.c` is compiled by NOTHING in this repository and by no CI job.
# `crates/kayfabe-qemu-raw/tests/shim_logic.rs` says the C half's acceptance "needs a
# hypervisor binary and gets one" — this phase is that sentence made executable.
phase qom-shim "qemusrc" \
  "the QOM shim overlay builds INTO a real hypervisor tree (the C half of the L2 adapter compiles)" \
  './scripts/build_qom_shim.sh "$KAYFABE_QEMU_SRC"'

# =====================================================================================
# Phase bodies that are more than one line
# =====================================================================================

# ★★★ THE TARGET CENSUS — derived universe, positional comparison.
#
# `cargo metadata` is the universe: every workspace target with `test: true`, plus every
# package with `doctest: true`. The observed set is parsed out of the hardware run's own
# output (`Running … (target/debug/deps/<name>-<hash>)` and `Doc-tests <pkg>`). A target in
# the universe that produced no output line NEVER RAN — which is the exact shape of the
# defect this script exists to catch, one level above the test.
census_targets() {
  [ -f "$LOGDIR/test-hardware.log" ] || {
    echo "★ no hardware-run log at $LOGDIR/test-hardware.log — the census has nothing to read."
    echo "  Run the test-hardware phase first (this phase deliberately does not run the suite"
    echo "  itself: a census that produces its own evidence proves nothing about the real run)."
    return 1
  }
  python3 - "$LOGDIR/test-hardware.log" "$TARGET_UNIVERSE_FLOOR" <<'PY'
import json, re, subprocess, sys
from collections import Counter

log_path, floor = sys.argv[1], int(sys.argv[2])
meta = json.loads(subprocess.run(
    ["cargo", "metadata", "--no-deps", "--format-version", "1"],
    capture_output=True, text=True, check=True).stdout)
members = set(meta["workspace_members"])

want = Counter()
want_doc = Counter()
owner = {}
for pkg in meta["packages"]:
    if pkg["id"] not in members:
        continue
    for tgt in pkg["targets"]:
        if tgt.get("test"):
            name = tgt["name"].replace("-", "_")
            want[name] += 1
            owner.setdefault(name, []).append(f'{pkg["name"]}:{"+".join(tgt["kind"])}')
        # ★ Normalised to underscores on BOTH sides. `cargo metadata` reports the PACKAGE
        # name (`kayfabe-vmm-qemu`); cargo's own output names the CRATE
        # (`Doc-tests kayfabe_vmm_qemu`). Comparing them raw reported all 22 doc-test
        # packages as never having run — on a run in which every one of them did. Caught by
        # the census's first real execution, which is also the proof it is not a constant
        # function.
        if tgt.get("doctest"):
            want_doc[pkg["name"].replace("-", "_")] += 1

if sum(want.values()) < floor:
    print(f"★★★ TARGET UNIVERSE TRUNCATED — cargo metadata yielded "
          f"{sum(want.values())} test targets, floor is {floor}.")
    print("This is NOT 'a test failed'. It is the UNIVERSE being suspect: the census "
          "would report 'everything ran' over a set that had quietly shrunk.")
    sys.exit(1)

seen, seen_doc = Counter(), Counter()
run_re = re.compile(r"Running .*\(target/[^/]+/deps/([A-Za-z0-9_]+)-[0-9a-f]+\)")
doc_re = re.compile(r"^\s*Doc-tests (\S+)")
with open(log_path, errors="replace") as fh:
    for line in fh:
        m = run_re.search(line)
        if m:
            seen[m.group(1)] += 1
        m = doc_re.match(line)
        if m:
            seen_doc[m.group(1).replace("-", "_")] += 1

missing = []
for name, n in sorted(want.items()):
    if seen[name] < n:
        missing.append(f"  test target {name!r} — expected {n} run(s), saw {seen[name]} "
                       f"[{', '.join(owner[name])}]")
for name, n in sorted(want_doc.items()):
    if seen_doc[name] < n:
        missing.append(f"  doc-tests for package {name!r} — expected {n}, saw {seen_doc[name]}")

print(f"universe: {sum(want.values())} test targets + {sum(want_doc.values())} doc-test "
      f"packages (derived from cargo metadata, floor {floor})")
print(f"observed: {sum(seen.values())} test-binary runs + {sum(seen_doc.values())} doc-test runs")
if missing:
    print("★★★ TARGETS IN THE UNIVERSE THAT PRODUCED NO OUTPUT — they did not run:")
    print("\n".join(missing))
    sys.exit(1)
print("every derived target produced output.")
PY
}

# ★★★ THE GATE CENSUS — families derived from the markers, not named here.
#
# The repository's convention is that a capability-gated test prints
# `<FAMILY>-GATE: RAN <test>` or `<FAMILY>-GATE: SKIPPED <test> — <reason>` on BOTH arms,
# and that slow-gated tests print `SKIPPED (slow): <test> — …`. This function does not know
# which families exist. It greps for the shape, reports every family it finds with its
# ran/skipped split, and fails if ANY test skipped — because the phase only ran at all on a
# box whose profile promised the capabilities.
census_gates() {
  local log="$LOGDIR/test-hardware.log"
  [ -f "$log" ] || { echo "★ no hardware-run log at $log — the census cannot be believed"; return 1; }
  local rc=0
  local fams
  fams=$(grep -oE '\b[A-Z0-9][A-Z0-9-]*-GATE: (RAN|SKIPPED)' "$log" | sed 's/-GATE:.*//' | sort -u)
  if [ -z "$fams" ]; then
    echo "★★★ NO GATE MARKERS AT ALL in the run log. The gated tests either vanished or"
    echo "stopped announcing themselves; either way nothing red would have said so."
    return 1
  fi
  # ★ NOT anchored with `^`. The markers are written straight to an unbuffered stderr while
  # libtest writes its own progress to stdout, so a marker can land mid-line. CI's own
  # reached-count steps grep unanchored for exactly this reason; an anchored count here
  # would silently under-report and make a skipping run look clean.
  local fam ran skipped
  for fam in $fams; do
    ran=$(grep -c "${fam}-GATE: RAN" "$log")
    skipped=$(grep -c "${fam}-GATE: SKIPPED" "$log")
    printf '  %-16s RAN %-5s SKIPPED %s\n' "$fam" "$ran" "$skipped"
    if [ "$skipped" -gt 0 ]; then
      echo "  ★ ${fam}-gated tests SKIPPED on a box whose profile promised the capability:"
      grep "${fam}-GATE: SKIPPED" "$log" | sed 's/^/      /' | head -60
      rc=1
    fi
    if [ "$ran" -eq 0 ]; then
      echo "  ★ ${fam}-gated tests: NONE ran. The family is present in the log but every"
      echo "    member skipped or the RAN marker stopped being printed."
      rc=1
    fi
  done
  local slow
  slow=$(grep -c 'SKIPPED (slow):' "$log")
  printf '  %-16s %s\n' "slow-gated" "SKIPPED $slow (KAYFABE_SLOW=1 was set, so this must be 0)"
  if [ "$slow" -gt 0 ]; then
    grep 'SKIPPED (slow):' "$log" | sed 's/^/      /' | head -40
    rc=1
  fi
  local ignored
  ignored=$(grep -cE '^test .* \.\.\. ignored' "$log")
  printf '  %-16s %s (allowance %s)\n' "#[ignore]d" "$ignored" "$IGNORED_ALLOWANCE"
  if [ "$ignored" -gt "$IGNORED_ALLOWANCE" ]; then
    echo "  ★ more #[ignore]d tests than the pinned allowance — an ignored test is"
    echo "    invisible in a summary and impossible to count, which is why this repo"
    echo "    uses loud runtime skips instead."
    grep -E '^test .* \.\.\. ignored' "$log" | sed 's/^/      /'
    rc=1
  fi
  return "$rc"
}

# ★★★ THE WORKSPACE CENSUS — the level above the target census.
#
# `cargo test --workspace` is a statement about ONE workspace. A `Cargo.toml` carrying its
# own `[workspace]` table is a *root*, and its members are unreachable from any command run
# at the repository root: not `--workspace`, not `--all-targets`, not any CI job. That is
# how `crates/kayfabe-abi/gen`'s 22 unit tests came to have never run anywhere.
#
# Discovery is by grep over every `Cargo.toml` in the tree (target dirs excluded), so a new
# detached sub-project is FOUND. It is compared against a pinned list, so being found is not
# the same as being covered: an unlisted workspace fails this phase until a phase runs it.
# ★★★ The fuzz-target universe, derived once and shared by both fuzz phases.
#
# `cargo fuzz list` reads the `[[bin]]` table in `fuzz/Cargo.toml`, which is the same
# table `cargo fuzz run` dispatches on — so this cannot drift from what is runnable. It
# is deliberately NOT a `ls fuzz/fuzz_targets/*.rs`: a file present but unregistered is
# not a target, and counting it would report a coverage this script cannot deliver.
fuzz_targets() {
  ( cd fuzz && cargo +nightly fuzz list )
}

# Run every target for `KAYFABE_FUZZ_SECS` (default 180) each. Serial on purpose: the
# phase's job is to prove each target EXECUTES, and a parallel run makes an OOM or a
# timeout ambiguous about which target produced it.
run_fuzz_all() {
  local t rc=0 n=0
  for t in $(fuzz_targets); do
    n=$((n + 1))
    echo "--- fuzz-run: $t (${KAYFABE_FUZZ_SECS:-180}s)"
    ( cd fuzz && cargo +nightly fuzz run "$t" -- \
        -max_total_time="${KAYFABE_FUZZ_SECS:-180}" -timeout=60 ) || rc=1
  done
  # A zero universe is a red run, not a clean one: it means the list stopped describing
  # the tree, which is exactly the failure mode a derived universe exists to make loud.
  [ "$n" -gt 0 ] || { echo "★★★ cargo fuzz list named NO targets"; rc=1; }
  echo "fuzz-run: $n target(s) executed"
  return $rc
}

# Replay each target's committed corpus + artifacts exactly once (`-runs=0`).
replay_fuzz_all() {
  local t rc=0 n=0 files total=0
  for t in $(fuzz_targets); do
    n=$((n + 1))
    files=0
    [ -d "fuzz/corpus/$t" ]    && files=$((files + $(find "fuzz/corpus/$t"    -type f | wc -l)))
    [ -d "fuzz/artifacts/$t" ] && files=$((files + $(find "fuzz/artifacts/$t" -type f | wc -l)))
    total=$((total + files))
    if [ "$files" -eq 0 ]; then
      # ⚠ Loud, and RED. An empty corpus directory replays nothing and libFuzzer exits 0
      # for it — the counted-but-never-passes shape. A target with no committed inputs is
      # a gap in the regression set, so it is reported as one.
      echo "★★★ fuzz-corpus-replay: '$t' has ZERO committed corpus/artifact files"
      echo "  Nothing is replayed for it, so a re-introduced crash on a known-bad input"
      echo "  would be invisible. Commit a minimised corpus under fuzz/corpus/$t/."
      rc=1
      continue
    fi
    echo "--- fuzz-corpus-replay: $t ($files file(s))"
    ( cd fuzz && cargo +nightly fuzz run "$t" \
        "corpus/$t" "artifacts/$t" -- -runs=0 ) || rc=1
  done
  [ "$n" -gt 0 ] || { echo "★★★ cargo fuzz list named NO targets"; rc=1; }
  echo "fuzz-corpus-replay: $total file(s) replayed across $n target(s)"
  return $rc
}

census_workspaces() {
  local found rc=0
  found=$(find . -name Cargo.toml \
            -not -path '*/target/*' -not -path '*/mutants.out/*' -not -path './.git/*' \
            -not -path './.claude/*' \
            -exec grep -l '^\[workspace\]' {} + \
          | sed 's|/Cargo.toml$||; s|^\./||; s|^Cargo.toml$|.|' | sort)
  echo "cargo workspace roots discovered:"
  echo "$found" | sed 's/^/  /'
  local w
  for w in $found; do
    case " $HANDLED_WORKSPACES " in
      *" $w "*) ;;
      *)
        echo "★★★ UNCOVERED CARGO WORKSPACE: $w"
        echo "  Its Cargo.toml carries its own [workspace] table, so NOTHING run at the"
        echo "  repository root reaches its tests — not cargo test --workspace, not any CI"
        echo "  job. Add a phase to scripts/run_full_suite.sh that runs them, and add the"
        echo "  path to HANDLED_WORKSPACES in the same commit."
        rc=1 ;;
    esac
  done
  for w in $HANDLED_WORKSPACES; do
    echo "$found" | grep -qx "$w" || {
      echo "★ HANDLED_WORKSPACES names '$w', which is no longer a workspace root."
      echo "  A stale entry means the list has stopped describing the tree; drop it."
      rc=1
    }
  done
  return "$rc"
}

# The mutation campaign, with the workflow's own two guards (ICE detection, threshold).
run_mutants() {
  local threshold="${MUTATION_THRESHOLD_PCT:-91}"
  KAYFABE_SLOW=1 CARGO_INCREMENTAL=0 cargo mutants --test-workspace true \
    -j "${KAYFABE_MUTANTS_JOBS:-8}" --timeout 1800 --build-timeout 900 \
    -f 'crates/*/src/**/*.rs' || true
  if grep -rlq "thread 'rustc'" mutants.out/log/ 2>/dev/null; then
    echo "★ A mutant build ICE'd — cargo-mutants files that as 'unviable', which removes it"
    echo "  from the denominator and INFLATES the score. Do not trust this campaign."
    return 1
  fi

  # ★★★ THE SAME MECHANISM, A SECOND CAUSE — and it fired for real.
  #
  # `[measured]` 2026-08-03, vast 46529600, 33,374 s: this phase printed
  # `mutation score 100% (killed 1936 / viable 1936), threshold 91%` and `ok`, while its own
  # output carried **15** `No space left on device` errors, `Error: write outcomes.json`, and
  # `Found 8738 mutants to test`. **6802 mutants — 78% — were unaccounted for**, and there
  # were ZERO `MISSED`/`TIMEOUT` lines.
  #
  # The ICE guard above understood the mechanism exactly — *"removes it from the denominator
  # and INFLATES the score"* — and covered only the ICE trigger. ENOSPC does the same thing,
  # and worse: it stops the RESULT FILES being written at all. `missed.txt` was never created,
  # `n()` read its absence as **0**, and `viable = killed + 0 = killed` forces the score to
  # **exactly 100%**. ⊘ A perfect score here is the ALARM, not the achievement.
  #
  # ⇒ This is the project's own FIFTH LIMIT — *an empty capture is evidence of NOTHING, not
  # evidence of emptiness* — inside its own harness. Absence must never read as zero.
  local found
  if [ ! -s mutants.out/outcomes.json ]; then
    echo "★★★ NO USABLE mutants.out/outcomes.json — cargo-mutants could not write its own"
    echo "  authoritative record, so THERE IS NO RESULT to score. Do not read the numbers"
    echo "  below as a campaign; the usual cause is the disk filling mid-run (df -h)."
    return 1
  fi
  if grep -rqi "No space left on device" mutants.out/ 2>/dev/null; then
    echo "★★★ ENOSPC INSIDE THE CAMPAIGN. Mutants whose build or run died of a full disk are"
    echo "  DROPPED FROM THE DENOMINATOR, which drives the score UP. The campaign measured a"
    echo "  universe that collapsed under it. Free space and re-run; do not trust this."
    return 1
  fi

  local n caught missed to killed viable score
  # ⊘ Absence is UNMEASURED, not zero — that substitution is what produced the 100% above.
  n() {
    if [ -f "mutants.out/$1.txt" ]; then wc -l < "mutants.out/$1.txt"; else echo MISSING; fi
  }
  caught=$(n caught); missed=$(n missed); to=$(n timeout)
  for f in caught missed timeout; do
    [ -f "mutants.out/$f.txt" ] || {
      echo "★★★ mutants.out/${f}.txt DOES NOT EXIST. Reading that as \"zero ${f}\" is exactly"
      echo "  how a truncated campaign scores 100%. The campaign did not complete."
      return 1
    }
  done
  killed=$((caught + to)); viable=$((killed + missed))
  [ "$viable" -gt 0 ] || { echo "★ zero viable mutants — the campaign measured nothing"; return 1; }

  # ★ The universe check: cargo-mutants says how many it FOUND. A large unexplained shortfall
  # between found and scored means the campaign did not test what it set out to test — and
  # the ratio is printed either way, because a number beside the bar it cleared is evidence
  # and a number on its own is decoration.
  found=$(grep -ohE "Found [0-9]+ mutants to test" mutants.out/*.txt mutants.out/log/* 2>/dev/null \
            | grep -ohE "[0-9]+" | head -1)
  if [ -n "$found" ] && [ "$found" -gt 0 ]; then
    local pct=$(( viable * 100 / found ))
    echo "mutants found ${found} / scored ${viable} (${pct}%)"
    if [ "$pct" -lt "${MUTATION_COVERAGE_MIN_PCT:-80}" ]; then
      echo "★★★ ONLY ${pct}% of the mutants cargo-mutants found were scored. The rest did not"
      echo "  survive and get counted — they VANISHED, and vanishing inflates the score."
      return 1
    fi
  else
    echo "★ could not read the found-mutant count; the universe check did not run."
  fi

  score=$(( killed * 100 / viable ))
  echo "mutation score ${score}% (killed ${killed} / viable ${viable}), threshold ${threshold}%"
  [ "$score" -ge "$threshold" ]
}

# =====================================================================================
# Argument handling
# =====================================================================================
PROFILE=""
ONLY=""
ALLOW_SKIP=""
LIST=0
while [ $# -gt 0 ]; do
  case "$1" in
    --profile)    PROFILE="${2:?--profile needs a value}"; shift 2 ;;
    --only)       ONLY="${2:?--only needs a comma-separated phase list}"; shift 2 ;;
    --allow-skip) ALLOW_SKIP="$ALLOW_SKIP ${2:?--allow-skip needs a phase name}"; shift 2 ;;
    --list)       LIST=1; shift ;;
    -h|--help)    sed -n '2,60p' "$0"; exit 0 ;;
    *)            echo "★ unknown flag $1 (--profile --only --allow-skip --list)"; exit 2 ;;
  esac
done

if [ -z "$PROFILE" ]; then
  if probe_gpu; then PROFILE=gpu-box; else PROFILE=ci; fi
  AUTOPROFILE=" (auto-detected)"
else
  AUTOPROFILE=""
fi
REQUIRED="$(profile_reqs "$PROFILE")"

# ---- THE PHASE FLOOR, checked before a single phase runs ----------------------------
if [ "${#PHASE_NAMES[@]}" -lt "$PHASE_FLOOR" ]; then
  echo "★★★ PHASE LIST TRUNCATED — ${#PHASE_NAMES[@]} phase(s), floor is $PHASE_FLOOR."
  echo "Nothing was run. This is the gate RUNNER being suspect, not a gate failing:"
  echo "a runner that reports success having run nothing hides every other defect."
  echo "If phases were deliberately removed, LOWER PHASE_FLOOR in the same commit."
  exit 3
fi

if [ "$LIST" -eq 1 ]; then
  echo "profile: $PROFILE — requires: ${REQUIRED:-(nothing)}"
  printf '%-24s %-34s %s\n' PHASE REQUIRES WHAT
  for i in "${!PHASE_NAMES[@]}"; do
    printf '%-24s %-34s %s\n' "${PHASE_NAMES[$i]}" "${PHASE_REQS[$i]:-—}" "${PHASE_WHAT[$i]}"
  done
  exit 0
fi

LOGDIR="${KAYFABE_SUITE_LOGDIR:-$REPO/target/full-suite}"
mkdir -p "$LOGDIR"
export LOGDIR

# =====================================================================================
# Probe every requirement ONCE, up front, and print the table
# =====================================================================================
echo "======================================================================"
echo " kayfabe — full suite"
echo " repo      $REPO"
echo " revision  $(git rev-parse --short HEAD 2>/dev/null || echo '(not a git tree)')$(git diff --quiet 2>/dev/null || echo ' +dirty')"
echo " host      $(uname -srm) / $(nproc) cores"
echo " profile   $PROFILE$AUTOPROFILE"
echo " logs      $LOGDIR"
echo "======================================================================"
echo
# =====================================================================================
# ★★★ THE BOX, measured before the tree
# =====================================================================================
# Everything printed here answers one question a reader will otherwise ask about a red run:
# *was it the code, or was it the machine?* It is printed unconditionally — a box-state line
# that only appears when something is wrong is a line nobody learns to read.
echo "BOX STATE"
free_mb() { df -Pm "$1" 2>/dev/null | awk 'NR==2 {print $4}'; }
disk_free=$(free_mb "$REPO")
tmp_free=$(free_mb "${TMPDIR:-/tmp}")
printf '  %-14s %s MB free on the repo filesystem, %s MB on %s\n' \
  "disk" "${disk_free:-?}" "${tmp_free:-?}" "${TMPDIR:-/tmp}"

# ★ A dirty tree is not an error — it is the normal state of a working branch. It is
# reported because a gate that greps the tree (the boundary, vocabulary, unsafe-surface and
# ABI-quarantine gates all do) measures whatever is ON DISK, so an untracked file from
# another writer is a real input to a real gate. A red run in a dirty tree needs that fact
# beside it, not discovered afterwards.
dirty=$(git status --porcelain 2>/dev/null | wc -l)
untracked=$(git status --porcelain 2>/dev/null | grep -c '^??' || true)
printf '  %-14s %s change(s) in the working tree, %s untracked\n' "git" "$dirty" "$untracked"
if [ "${dirty:-0}" -gt 0 ]; then
  echo "  ★ The tree is not clean. The boundary/vocabulary/unsafe-surface/ABI gates grep"
  echo "    the tree AS IT IS, so a file that belongs to someone else is a real input to a"
  echo "    real gate: a failure below may be about code that is in no branch. First 10:"
  git status --porcelain 2>/dev/null | head -10 | sed 's/^/      /'
fi
if [ -n "${disk_free:-}" ] && [ "$disk_free" -lt "$DISK_REFUSE_MB" ]; then
  echo
  echo " ★★★ REFUSING TO RUN — ${disk_free} MB free, below the ${DISK_REFUSE_MB} MB floor."
  echo " Nothing was run, and this says NOTHING about the tree. A build that dies of ENOSPC"
  echo " reports a compiler error, and a compiler error reads as a code defect; that misread"
  echo " cost a whole verification cycle on 2026-07-30. Free space and re-run."
  exit 4
fi
if [ -n "${disk_free:-}" ] && [ "$disk_free" -lt "$DISK_WARN_MB" ]; then
  echo "  ★ Below the ${DISK_WARN_MB} MB comfort bar. A full run builds the workspace, an"
  echo "    aarch64 check, a release archive, an instrumented std and possibly a hypervisor."
  echo "    If a phase below dies with a write error, suspect the disk before the code."
fi
echo
echo "REQUIREMENTS"
declare -A HAVE=()
for r in $ALL_REQS; do
  if "probe_$r"; then HAVE[$r]=1; else HAVE[$r]=0; fi
  case " $REQUIRED " in *" $r "*) expected="required by profile $PROFILE" ;; *) expected="not required by profile $PROFILE" ;; esac
  if [ "${HAVE[$r]}" = 1 ]; then
    printf '  %-14s PRESENT   (%s)\n' "$r" "$expected"
  else
    printf '  %-14s ABSENT    (%s)\n' "$r" "$expected"
    printf '  %-14s           needs: %s\n' "" "$("fix_$r")"
  fi
done
echo

# =====================================================================================
# Run
# =====================================================================================
RAN_OK=(); FAILED=(); SKIPPED=(); SKIP_REASON=(); ACKED=()
for i in "${!PHASE_NAMES[@]}"; do
  name="${PHASE_NAMES[$i]}"
  reqs="${PHASE_REQS[$i]}"
  cmd="${PHASE_CMDS[$i]}"

  if [ -n "$ONLY" ] && ! echo ",$ONLY," | grep -q ",$name,"; then
    continue
  fi

  unmet=""
  for r in $reqs; do
    [ "${HAVE[$r]}" = 1 ] || unmet="$unmet $r"
  done
  case " $ALLOW_SKIP " in *" $name "*) acked=1 ;; *) acked=0 ;; esac

  if [ -n "$unmet" ] || [ "$acked" = 1 ]; then
    if [ "$acked" = 1 ]; then
      ACKED+=("$name")
      echo "=== $name — ACKNOWLEDGED SKIP (--allow-skip $name)"
      continue
    fi
    reason=""
    for r in $unmet; do reason="$reason${reason:+; }$r — $("fix_$r")"; done
    SKIPPED+=("$name"); SKIP_REASON+=("$reason")
    echo "=== $name — SKIPPED, unmet:$unmet"
    continue
  fi

  echo "=== $name — ${PHASE_WHAT[$i]}"
  start=$(date +%s)
  # ★ `pipefail` is on for the whole script, so `$?` of this pipeline is the PHASE's status
  # and not `tee`'s. Written as a bare pipeline followed by `rc=$?` rather than as an `if`:
  # the assignments in an `if`'s branches are themselves commands and RESET `PIPESTATUS`,
  # which is how the first draft of this loop reported every phase as passing.
  ( eval "$cmd" ) 2>&1 | tee "$LOGDIR/$name.log"
  rc=$?
  took=$(( $(date +%s) - start ))
  if [ "$rc" -eq 0 ]; then
    RAN_OK+=("$name"); echo "    ok (${took}s)"
  else
    FAILED+=("$name"); echo "    ★ FAILED (rc=$rc, ${took}s) — $LOGDIR/$name.log"
  fi
done

# =====================================================================================
# ★★★ THE LEDGER
# =====================================================================================
echo
echo "======================================================================"
echo " LEDGER — profile $PROFILE"
echo "======================================================================"
printf ' RAN %d / FAILED %d / SKIPPED %d / ACKNOWLEDGED %d\n' \
  "${#RAN_OK[@]}" "${#FAILED[@]}" "${#SKIPPED[@]}" "${#ACKED[@]}"

# ★★ The box, again, beside the result. A failure list without the environment it was
# produced in is the thing that sends someone reading code for an hour over a full disk or a
# foreign file. Printed on green runs too, so it is a line people can read rather than an
# alarm they only meet in a crisis.
disk_end=$(free_mb "$REPO")
printf ' box:  %s MB free (was %s at start), %s working-tree change(s), revision %s\n' \
  "${disk_end:-?}" "${disk_free:-?}" "${dirty:-?}" \
  "$(git rev-parse --short HEAD 2>/dev/null || echo '?')"
if [ "${#FAILED[@]}" -gt 0 ]; then
  if [ -n "${disk_end:-}" ] && [ "$disk_end" -lt "$DISK_WARN_MB" ]; then
    echo " ★ …and the disk is below the comfort bar. Check the failing phase's log for a"
    echo "   write error before reading any of this as being about the code."
  fi
  if [ "${dirty:-0}" -gt 0 ]; then
    echo " ★ …and the tree is dirty. The grep-based gates measured what is on disk, which"
    echo "   includes those changes — including any that belong to another writer."
  fi
fi
echo
for n in "${RAN_OK[@]:-}";  do [ -n "$n" ] && printf '  RAN          %s\n' "$n"; done
for n in "${FAILED[@]:-}";  do [ -n "$n" ] && printf '  ★ FAILED     %s   (%s)\n' "$n" "$LOGDIR/$n.log"; done
for i in "${!SKIPPED[@]}"; do
  printf '  SKIPPED      %s\n' "${SKIPPED[$i]}"
  printf '               unmet: %s\n' "${SKIP_REASON[$i]}"
done
for n in "${ACKED[@]:-}"; do [ -n "$n" ] && printf '  ACKNOWLEDGED %s   (--allow-skip; the operator asserted this is deliberate)\n' "$n"; done

# ★ A skip is RED when the profile says this box should have satisfied the requirement.
# Without that rule the ledger is circular — "the thing was missing, so skipping was fine"
# is true on every machine and rules nothing out.
rc=0
[ "${#FAILED[@]}" -gt 0 ] && rc=1
should_have=()
for i in "${!SKIPPED[@]}"; do
  name="${SKIPPED[$i]}"
  for j in "${!PHASE_NAMES[@]}"; do [ "${PHASE_NAMES[$j]}" = "$name" ] && reqs="${PHASE_REQS[$j]}"; done
  for r in $reqs; do
    if [ "${HAVE[$r]}" != 1 ]; then
      case " $REQUIRED " in *" $r "*) should_have+=("$name needs $r, which profile $PROFILE says this box has") ;; esac
    fi
  done
done
if [ "${#should_have[@]}" -gt 0 ]; then
  echo
  echo " ★★★ SKIPPED FOR A REASON THIS BOX WAS SUPPOSED TO SATISFY:"
  for s in "${should_have[@]}"; do echo "   - $s"; done
  echo " A clean exit must mean \"everything this box can run, ran\". It did not."
  rc=1
fi

echo
if [ "$rc" -eq 0 ]; then
  echo " ✔ EVERYTHING THIS BOX CAN RUN, RAN."
else
  echo " ✘ NOT A CLEAN RUN — see the ledger above."
fi
exit "$rc"
