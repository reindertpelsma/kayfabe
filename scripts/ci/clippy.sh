#!/usr/bin/env bash
# Compiler/denied-Clippy errors remain fatal; new warning sites fail the debt gate.
set -euo pipefail
cd "$(dirname "$0")/../.."
log=$(mktemp -t kayfabe-clippy.XXXXXX.jsonl)
trap 'rm -f "$log"' EXIT
if cargo clippy --workspace --all-targets --locked --message-format=json >"$log"; then
    python3 scripts/ci/debt.py clippy --log "$log"
else
    python3 -c 'import json,sys; [print(r["message"]["rendered"], end="") for l in open(sys.argv[1]) if (r:=json.loads(l)).get("reason")=="compiler-message" and r["message"]["level"]=="error"]' "$log"
    exit 1
fi
