#!/usr/bin/env bash
# Collect the failing-test NAME set. Writes a start marker and an exit-status line
# (CLAUDE.md: "zero bytes is not 'not yet'; it is a state that needs its own check").
out="$1"
echo "START $(date -Is) rev=$(git -C /workspace/kf-w753 rev-parse --short HEAD)" > "$out"
cd /workspace/kf-w753
CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 \
  cargo test --workspace --no-fail-fast >> "$out" 2>&1
rc=$?
echo "TERMINATOR exit=$rc $(date -Is)" >> "$out"
