#!/usr/bin/env bash
# Optional native observation stage; completed calibration still refuses guest admission.
set -euo pipefail
host=$1 arch=$2 repo=$3 directory=$4
ladder=${KF_NATIVE_LADDER:-$repo/target/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder}
probe=${KF_NATIVE_CTL:-$repo/target/release/kf-ctl-probe}
exec python3 "$repo/scripts/bench/diesweep/native_sweep_proof.py" "$host" "$arch" "$repo" "$directory" "$ladder" "$probe"
