#!/usr/bin/env bash
# ★★★ PROVISION A FRESH BENCH BOX — the recipe, so nobody rebuilds one from memory.
#
# Run `host_preflight.sh` FIRST. If it says HOST_FAULT, destroy the instance and rent
# another; provisioning a broken box wastes the time the preflight exists to save.
#
# ⚠ THE ONE STEP THAT IS NOT OBVIOUS AND FAILS LOUDLY-BUT-MISLEADINGLY:
#   `rustup target add x86_64-unknown-linux-musl`
# `kayfabe-isolate-host/build.rs` builds the embedded isolate as a **static musl binary**
# (`:164`), so without the musl std the WHOLE WORKSPACE fails with a bare
#   error[E0463]: can't find crate for `std`
# on `kayfabe-util` — a crate that has nothing to do with musl and names no target. The
# build.rs says so itself at `:41-44` ("every CI job simply declares the musl target
# alongside its own"); the error surfaces far from the cause.
set -euo pipefail
echo "PROVISION_START $(date -Is)"

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq build-essential pkg-config libssl-dev git curl clang lld python3

if ! command -v cargo >/dev/null 2>&1; then
  curl -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal
fi
. "$HOME/.cargo/env"
rustup target add x86_64-unknown-linux-musl   # ⚠ see the header — not optional

[ -d ~/kayfabe ] || git clone -q https://github.com/reindertpelsma/kayfabe.git ~/kayfabe
cd ~/kayfabe
git fetch -q origin && git reset -q --hard origin/master
echo "HEAD=$(git rev-parse --short HEAD)"

# ⊘ Do NOT pipe cargo into tail: `cargo build | tail` makes $? the status of TAIL, which
#    always succeeds, so a FAILED build reports success. That exact bug produced a green
#    provisioning run over a workspace that had not compiled (2026-09-06). Capture the
#    status directly and read the log from a file.
set +e
cargo build --workspace > /tmp/kayfabe-build.log 2>&1
CARGO_RC=$?
set -e
echo "CARGO_RC=$CARGO_RC"
if [ "$CARGO_RC" -ne 0 ]; then
  echo "--- build errors ---"
  grep -E "^error|panicked at|could not compile" /tmp/kayfabe-build.log | head -20
fi
echo "PROVISION_DONE rc=$CARGO_RC $(date -Is)"
exit "$CARGO_RC"
