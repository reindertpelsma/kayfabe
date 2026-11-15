#!/bin/bash
# merge_check.sh — the merge bar for promoting a revision to master/v3, run ON a provisioned box.
#   bash merge_check.sh [options] <branch> <tag>   (log: /root/prov/<tag>.log, ends with an EXIT line)
# Bar (a LIGHT correctness check on any one GPU; [2026-10-26] it no longer repeats CI's crate tests):
# v3 gates 9/9, a kf3 build of THIS revision, the raw client + fast guest rebuilt, the 30-arm
# thin-guest suite at budget 180 -> 30/30, and the channel-birth census (birth_census.sh,
# THE_CONSTRAINTS §30): every channel the suite and the gates birth reads PRIVILEGED_CHANNEL=0, every
# arm births at least one, every CUDA thread cleared CAP_SYS_ADMIN. The GPU-free `cargo test` of the
# kf-* crates is CI's `stable` job (every commit); the only tests that differ on a box are the
# /dev/kvm-gated ones, an opt-in stage: KF_MERGE_KVM_TESTS=1 (kvm_tests.sh). Live gate run
# 40263112941 spent 31 minutes and ended rc=101 in the old all-crates run.
# A second opt-in stage, KF_MERGE_WINDOWS=1 (windows_gate.sh, LAST, after the census): Windows 11 LTSC on kf3,
# installed once per box, the stock 580.88 driver with GSP forced, a signed-in desktop, nvidia-smi, a CUDA
# matmul and a D3D11 readback, a bounded idle, zero resets. It adds an install of about 20-45 minutes and
# ends with WINDOWS_GATE_PASS, or FAIL (the guest failed) or RETRY (host, harness or rental fault: re-run on a fresh box); every non-PASS verdict exits non-zero.
# Options (2026-11-15, OWNER_RULINGS section AR; no option = the full bar, exactly as before):
#   --no-fetch     resolve <branch> in the local repository; never `git fetch origin`
#   --tree PATH    test the checkout at PATH as it is: no fetch, no worktree, NO strip (the caller owns that;
#                  <branch> is only a label there, e.g. "-"). Nothing here deletes in it; a tree that was not
#                  stripped is therefore NOT the merge bar's tree (OWNER_RULINGS AQ), so a run on it is no merge claim.
#   --gates --kf3 --host --fast --census   run ONLY the named sections (any combination, always in bar order).
#                  gates = v3 gates; kf3 = build_kf3; host = raw client + bare-metal suite; fast = fast guest
#                  build + suite; census = birth census (reads this tag's gates log and suite output, so run
#                  it with the same tag after those, or it fails closed). The KVM and Windows opt-ins are unchanged.
# ⊘ Never hold /tmp/kayfabe-fastguest.lock across fast_suite: run_fast_guest takes it per arm.
# Never reuse a previous initrd after a build failure, delete a shared checkout, or hide a
# failed cargo behind a successful log filter. Every stage fails closed and keeps its log.
set -euo pipefail
NOFETCH=0 TREE="" SECT=" "
pos=()
while [ $# -gt 0 ]; do
    case "$1" in
        --no-fetch) NOFETCH=1 ;;
        --tree) TREE=${2:?--tree needs a path}; shift ;;
        --gates|--kf3|--host|--fast|--census) SECT="$SECT${1#--} " ;;
        --) shift; pos+=("$@"); break ;;
        -) pos+=("$1") ;;
        -*) echo "unknown option $1" >&2; exit 2 ;;
        *) pos+=("$1") ;;
    esac
    shift
done
[ "${#pos[@]}" -eq 2 ] || { echo "usage: merge_check.sh [options] <branch> <tag>" >&2; exit 2; }
B=${pos[0]}; T=${pos[1]}
[ -z "$TREE" ] || { [ -d "$TREE/.git" ] || [ -f "$TREE/.git" ]; } || { echo "--tree $TREE is not a git checkout" >&2; exit 2; }
want() { [ "$SECT" = " " ] || [[ "$SECT" == *" $1 "* ]]; }
[[ "$T" =~ ^[a-zA-Z0-9][a-zA-Z0-9._-]*$ ]] || { echo "invalid tag" >&2; exit 2; }
SOURCE=$(cd "$(dirname "$0")/../../.." && pwd)
PROV=${KF_PROV_DIR:-/root/prov}
mkdir -p "$PROV"
[ ! -e "$PROV/$T.log" ] || { echo "refusing to overwrite $PROV/$T.log" >&2; exit 2; }
exec >"$PROV/$T.log" 2>&1
on_exit() { local rc=$?; echo "EXIT rc=$rc $(date -Is)"; }
trap on_exit EXIT
echo "START $(date -Is)"
exec 8>/tmp/kayfabe-merge-check.lock
flock -n 8 || { echo "another merge check is running"; exit 2; }
export PATH="$PATH:$HOME/.cargo/bin"
export BENCH_DIR=${BENCH_DIR:-/workspace/bench} KF_DEVICE=kf3
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$BENCH_DIR/kf-verify-target}
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}
[[ "$CARGO_TARGET_DIR" = /* ]] || { echo "CARGO_TARGET_DIR must be absolute"; exit 2; }
if [ -n "$TREE" ]; then
    CHECKOUT=$(cd "$TREE" && pwd); REV=$(git -C "$CHECKOUT" rev-parse 'HEAD^{commit}')
else
    if [ "$NOFETCH" = 1 ]; then REV=$(git -C "$SOURCE" rev-parse "$B^{commit}")
    else git -C "$SOURCE" fetch -q origin "$B"; REV=$(git -C "$SOURCE" rev-parse 'FETCH_HEAD^{commit}'); fi
    mkdir -p "$BENCH_DIR/verify-worktrees"
    CHECKOUT=$(mktemp -d "$BENCH_DIR/verify-worktrees/$T.XXXXXX")
fi
# ⊘ [2026-10-26] The Vast onstart checks out a SPARSE tree (ci/vast-broker/src/onstart.js) and the
# payload points `origin` at that checkout, which never held the excluded blobs. A linked worktree
# must therefore be sparse too: git < 2.36 (the box image has 2.34) does not copy the patterns, and
# a full checkout would ask `origin` -- this very repository -- for blobs it does not have.
if [ -z "$TREE" ]; then
    git -C "$SOURCE" worktree add --detach --no-checkout "$CHECKOUT" "$REV"
    SPARSE_SRC=$(git -C "$SOURCE" rev-parse --absolute-git-dir)/info/sparse-checkout
    if [ -f "$SPARSE_SRC" ]; then
        SPARSE_DST=$(git -C "$CHECKOUT" rev-parse --absolute-git-dir)/info/sparse-checkout
        mkdir -p "$(dirname "$SPARSE_DST")"
        cp "$SPARSE_SRC" "$SPARSE_DST"
    fi
    git -C "$CHECKOUT" reset -q --hard "$REV"
fi
cd "$CHECKOUT"
echo "HEAD=$REV CHECKOUT=$CHECKOUT NOFETCH=$NOFETCH TREE=${TREE:-none} SECTIONS=${SECT:- all }"
# [2026-11-09] OWNER_RULINGS section AQ: the box tests the tree that is left after everything the
# commit's own .workerignore matches is deleted, so a test cannot depend on an ignored path. ONE
# matcher (scripts/ci/ignore_ladder.py `matches`, the one the stripped-tree CI job and the Worker's
# tree-key cross-check use). Fail closed: an unreadable or unknown-syntax file, an ignore set that
# would delete code under crates/ qemu/ cuda/ firmware/ tools/ or the pinned oracle paths, or a tree
# with nothing left, ends the run here (exit 2) and deletes nothing. The result is printed with the
# stripped-tree key (the Worker's key for this commit, see `ignore_ladder.py keycheck`).
[ -n "$TREE" ] || python3 -I scripts/ci/ignore_ladder.py strip --in-place || { echo "STRIP_FAILED: the .workerignore of $REV cannot be applied"; exit 2; }
# The strip must leave `git status` clean (stripped paths are skip-worktree): build_kf3.sh names the QEMU
# directory kf3-bins/<rev8>[-dirty] from it and the guest guard accepts only <rev8>. Loud, not silent.
[ -n "$TREE" ] || python3 -I scripts/ci/ignore_ladder.py strip-verify || { echo "STRIP_VERIFY_FAILED: the stripped checkout of $REV is not clean (see above)"; exit 2; }
# An oracle test that cannot run is a failure here, not a skip (hwref_is_current.rs).
export KF_REQUIRE_ORACLES=${KF_REQUIRE_ORACLES:-1}
echo "KF_REQUIRE_ORACLES=$KF_REQUIRE_ORACLES"
pk=()
for manifest in crates/kf-*/Cargo.toml; do
    crate=${manifest#crates/}; pk+=(-p "${crate%/Cargo.toml}")
done
# The census is a sparse-checkout guard: the build below needs every kf-* crate present.
[ "${#pk[@]}" -ge 36 ] || { echo "v3 crate census truncated"; exit 2; }
if [ "${KF_MERGE_KVM_TESTS:-0}" = 1 ]; then
    bash scripts/bench/box/kvm_tests.sh "$PROV" "$T"
fi
if want gates; then
    bash scripts/bench/v3_gates.sh "$PROV/${T}_gates.log" >"$PROV/${T}_gates.run" 2>&1
    echo "GATES_RC=0"
    # ★ D3 (2026-10-10): the summary carries gate 10, the micro-reservation probe (micro reservations are
    # the default): PASS, or FALLBACK (host RM refused them: the 4 KiB floor; loud, not a failure).
    grep -E '^V3_GATES_SUMMARY pass=9 fail=0 gate10=(PASS|FALLBACK)$' "$PROV/${T}_gates.log"
    grep -a '^GATE10_FALLBACK_ACTIVE' "$PROV/${T}_gates.log" || true
fi
if want kf3; then
    bash scripts/bench/build_kf3.sh "$BENCH_DIR/qemu-10.2.4" "$BENCH_DIR/qemu-build-kf3" >"$PROV/${T}_kf3.log" 2>&1
    echo "KF3_RC=0"
    tail -1 "$PROV/${T}_kf3.log"
fi
if want host || want fast; then
    cargo build -q --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder >"$PROV/${T}_client.log" 2>&1
    export CLIENT="$CARGO_TARGET_DIR/release/kayfabe-rm-ladder" KF_LADDER="$CARGO_TARGET_DIR/release/kayfabe-rm-ladder"
fi
if want host; then
    bash scripts/fastguest/bare_metal_suite.sh "${T}_host" 180 >"$PROV/${T}_host.log" 2>&1
    grep '^BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30$' "$PROV/${T}_host.log"
fi
if want fast; then
    KF_FROM_HOST=1 bash scripts/fastguest/build_fast_guest.sh "$BENCH_DIR/guest.qcow2" "$BENCH_DIR/fastguest" >"$PROV/${T}_fg.log" 2>&1
    echo "FG_RC=0"
    bash scripts/fastguest/fast_suite.sh "$T" 180 >"$PROV/${T}_suite.run" 2>&1
    echo "SUITE_RC=0"
    grep '^FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30$' "$BENCH_DIR/${T}_suite.out"
fi
if want census; then
    # ★ The census must first show it can fail (planted logs), then read this run's logs.
    bash scripts/bench/box/birth_census.sh --selftest >"$PROV/${T}_census_selftest.log" 2>&1 \
        || { tail -3 "$PROV/${T}_census_selftest.log"; exit 1; }
    tail -1 "$PROV/${T}_census_selftest.log"
    bash scripts/bench/box/birth_census.sh "$BENCH_DIR" "$T" "$PROV/${T}_gates.log" >"$PROV/${T}_births.log" 2>&1 \
        || { grep -a '^BIRTH_CENSUS_FAIL' "$PROV/${T}_births.log"; exit 1; }
    grep -a '^BIRTH_CENSUS_SUITE\|^BIRTH_CENSUS_GATES' "$PROV/${T}_births.log"
    grep -a '^BIRTH_CENSUS_OK' "$PROV/${T}_births.log"
fi
if [ "${KF_MERGE_WINDOWS:-0}" = 1 ]; then
    bash scripts/bench/box/windows_gate.sh "$T" "$PROV"
fi
