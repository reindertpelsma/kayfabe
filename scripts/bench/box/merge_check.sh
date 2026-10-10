#!/bin/bash
# merge_check.sh — the merge bar for promoting a revision to master/v3, run ON a provisioned box.
#   bash merge_check.sh <branch> <tag>        (log: /root/prov/<tag>.log, ends with an EXIT line)
# Bar: every kf-* crate test (--no-fail-fast: a red crate must not hide later crates — it once
# reported "483 tests" instead of ~1500), v3 gates 9/9, a kf3 build of THIS revision, the raw client
# + fast guest rebuilt, the 30-arm thin-guest suite at budget 180 → 30/30, and the channel-birth
# census (birth_census.sh, THE_CONSTRAINTS §30): every channel the suite and the gates birth reads
# PRIVILEGED_CHANNEL=0, every arm births at least one, every CUDA thread cleared CAP_SYS_ADMIN.
# ⊘ Never hold /tmp/kayfabe-fastguest.lock across fast_suite: run_fast_guest takes it per arm.
# Never reuse a previous initrd after a build failure, delete a shared checkout, or hide a
# failed cargo behind a successful log filter. Every stage fails closed and keeps its log.
set -euo pipefail
B=${1:?branch}; T=${2:?tag}
[[ "$T" =~ ^[a-zA-Z0-9][a-zA-Z0-9._-]*$ ]] || { echo "invalid tag" >&2; exit 2; }
SOURCE=$(cd "$(dirname "$0")/../../.." && pwd)
PROV=${KF_PROV_DIR:-/root/prov}
mkdir -p "$PROV"
[ ! -e "$PROV/$T.log" ] || { echo "refusing to overwrite $PROV/$T.log" >&2; exit 2; }
exec >"$PROV/$T.log" 2>&1
trap 'rc=$?; echo "EXIT rc=$rc $(date -Is)"' EXIT
echo "START $(date -Is)"
exec 8>/tmp/kayfabe-merge-check.lock
flock -n 8 || { echo "another merge check is running"; exit 2; }
export PATH="$PATH:$HOME/.cargo/bin"
export BENCH_DIR=${BENCH_DIR:-/workspace/bench} KF_DEVICE=kf3
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$BENCH_DIR/kf-verify-target}
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}
[[ "$CARGO_TARGET_DIR" = /* ]] || { echo "CARGO_TARGET_DIR must be absolute"; exit 2; }
git -C "$SOURCE" fetch -q origin "$B"
REV=$(git -C "$SOURCE" rev-parse 'FETCH_HEAD^{commit}')
mkdir -p "$BENCH_DIR/verify-worktrees"
CHECKOUT=$(mktemp -d "$BENCH_DIR/verify-worktrees/$T.XXXXXX")
git -C "$SOURCE" worktree add --detach "$CHECKOUT" "$REV"
cd "$CHECKOUT"
echo "HEAD=$REV CHECKOUT=$CHECKOUT"
pk=()
for manifest in crates/kf-*/Cargo.toml; do
    crate=${manifest#crates/}; pk+=(-p "${crate%/Cargo.toml}")
done
[ "${#pk[@]}" -ge 36 ] || { echo "v3 crate census truncated"; exit 2; }
cargo test -q --no-fail-fast "${pk[@]}" >"$PROV/${T}_tests.log" 2>&1
awk '/^test result:/{s+=$4; f+=$6} END{print "TESTS passed",s,"failed",f; if(s==0 || f!=0) exit 1}' "$PROV/${T}_tests.log"
bash scripts/bench/v3_gates.sh "$PROV/${T}_gates.log" >"$PROV/${T}_gates.run" 2>&1
echo "GATES_RC=0"
grep '^V3_GATES_SUMMARY pass=9 fail=0$' "$PROV/${T}_gates.log"
# ★ D3 (2026-10-10): gate 10, the micro-reservation probe (micro reservations are the default).
grep '^GATE10_VERDICT=PASS$' "$PROV/${T}_gates.log"
bash scripts/bench/build_kf3.sh "$BENCH_DIR/qemu-10.2.4" "$BENCH_DIR/qemu-build-kf3" >"$PROV/${T}_kf3.log" 2>&1
echo "KF3_RC=0"
tail -1 "$PROV/${T}_kf3.log"
cargo build -q --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder >"$PROV/${T}_client.log" 2>&1
export CLIENT="$CARGO_TARGET_DIR/release/kayfabe-rm-ladder" KF_LADDER="$CARGO_TARGET_DIR/release/kayfabe-rm-ladder"
bash scripts/fastguest/bare_metal_suite.sh "${T}_host" 180 >"$PROV/${T}_host.log" 2>&1
grep '^BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30$' "$PROV/${T}_host.log"
KF_FROM_HOST=1 bash scripts/fastguest/build_fast_guest.sh "$BENCH_DIR/guest.qcow2" "$BENCH_DIR/fastguest" >"$PROV/${T}_fg.log" 2>&1
echo "FG_RC=0"
bash scripts/fastguest/fast_suite.sh "$T" 180 >"$PROV/${T}_suite.run" 2>&1
echo "SUITE_RC=0"
grep '^FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30$' "$BENCH_DIR/${T}_suite.out"
# ★ The census must first show it can fail (planted logs), then read this run's logs.
bash scripts/bench/box/birth_census.sh --selftest >"$PROV/${T}_census_selftest.log" 2>&1 \
    || { tail -3 "$PROV/${T}_census_selftest.log"; exit 1; }
tail -1 "$PROV/${T}_census_selftest.log"
bash scripts/bench/box/birth_census.sh "$BENCH_DIR" "$T" "$PROV/${T}_gates.log" >"$PROV/${T}_births.log" 2>&1 \
    || { grep -a '^BIRTH_CENSUS_FAIL' "$PROV/${T}_births.log"; exit 1; }
grep -a '^BIRTH_CENSUS_SUITE\|^BIRTH_CENSUS_GATES' "$PROV/${T}_births.log"
grep -a '^BIRTH_CENSUS_OK' "$PROV/${T}_births.log"
