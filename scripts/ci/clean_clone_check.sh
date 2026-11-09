#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# GPU-free documented Rust build/test from committed sources only. No submodules,
# evidence downloads, driver installation, QEMU build or VM execution.
set -euo pipefail
HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
SOURCE="$(git -C "$HERE" rev-parse --show-toplevel)"
REF=""
fail() { echo "clean-clone: $*" >&2; exit 1; }
while [ "$#" -gt 0 ]; do
    case "$1" in
        --source) [ "$#" -ge 2 ] || fail "--source needs a URL or directory"; SOURCE="$2"; shift 2 ;;
        --ref) [ "$#" -ge 2 ] || fail "--ref needs a branch or tag"; REF="$2"; shift 2 ;;
        --help) echo "Usage: CARGO_TARGET_DIR=/absolute/empty-parent $0 [--source URL|DIR] [--ref BRANCH|TAG]"; exit 0 ;;
        *) fail "unknown argument: $1" ;;
    esac
done
[ -n "${CARGO_TARGET_DIR:-}" ] || fail "set CARGO_TARGET_DIR to a private absolute target parent"
[[ "$CARGO_TARGET_DIR" = /* ]] || fail "CARGO_TARGET_DIR must be absolute"
for tool in git python3 cargo nice mktemp df awk; do
    command -v "$tool" >/dev/null || fail "documented prerequisite missing: $tool"
done
check_disk() {
    local free_kib
    free_kib="$(df -Pk / | awk 'NR==2 {print $4}')"
    [[ "$free_kib" =~ ^[0-9]+$ ]] || fail "cannot measure free space on /"
    [ "$free_kib" -ge 6291456 ] || fail "less than 6 GiB free on /; stopping before build"
}
check_disk
mkdir -p -- "$CARGO_TARGET_DIR"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/kf-clean-clone.XXXXXXXX")"
TARGET=""
cleanup() {
    [ -z "$TARGET" ] || rm -rf -- "$TARGET"
    rm -rf -- "$WORK"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
TARGET="$(mktemp -d "$CARGO_TARGET_DIR/clean-clone.XXXXXXXX")"
export CARGO_TARGET_DIR="$TARGET"
# --no-local prevents hardlinks/local clone optimisations and makes --depth real
# for local repositories too. Only the selected committed ref enters the tree.
CLONE_ARGS=(clone --depth=1 --no-local --no-hardlinks)
[ -z "$REF" ] || CLONE_ARGS+=(--branch "$REF")
CLONE_ARGS+=(-- "$SOURCE" "$WORK/source")
START=$SECONDS
nice -n 19 git "${CLONE_ARGS[@]}" || fail "fresh shallow clone failed"
cd -- "$WORK/source"
[ "$(git rev-parse --is-shallow-repository)" = true ] || fail "clone is not shallow"
REV="$(git rev-parse HEAD)"
echo "CLEAN_CLONE_START rev=$REV"
nice -n 19 python3 "$HERE/clean_clone_inputs.py" "$PWD" || fail "documented steps or committed inputs are missing (no evidence is fetched)"
# Inherit these in nested firmware/isolate cargo builds too.
export CARGO_BUILD_JOBS=2
export KAYFABE_NO_KVM=1
unset KAYFABE_SLOW KAYFABE_ISOLATE_IMAGE_STUB
check_disk
nice -n 19 cargo build --workspace --release --locked -j2 || fail "documented release build failed; inspect the named prerequisite/input above"
BUILD_SECONDS=$((SECONDS - START))
check_disk
nice -n 19 cargo test --workspace --locked --no-fail-fast -j2 || fail "GPU-free tests failed; a missing runtime input must be committed, not restored from an evidence archive"
echo "CLEAN_CLONE_PASS rev=$REV build_seconds=$BUILD_SECONDS total_seconds=$((SECONDS - START))"
