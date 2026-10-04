#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# ★★★ G1d — the type-resolved address gate (v3-sec-rawaddr; audit S1-02;
# docs/design/V3_RAWADDR_PERIMETER.md §6): Clippy's `disallowed-methods` / `disallowed-types`
# (scripts/ci/address-clippy/clippy.toml) DENIED over every kf-* crate, with the perimeter
# (`*_unsafe.rs`, which carries the opt-out attribute) the only place they may appear.
#
# 1. Self-test first (a gate that reports zero must first report one): the known-positive fixture
#    must draw EXACTLY one diagnostic per `G1D`-marked line, and the perimeter fixture none.
# 2. Then the kf-* crates, all targets, in their own target dir (so the workspace clippy's cache and
#    this run's lint configuration never mix).
set -euo pipefail
cd "$(dirname "$0")/../.."
conf="$PWD/scripts/ci/address-clippy"
fx="$PWD/scripts/ci/fixtures/address_clippy"
tmp=$(mktemp -d -t kf-g1d.XXXXXX)
trap 'rm -rf "$tmp"' EXIT

lint() { # $1 = a fixture; prints the number of disallowed_* diagnostics
    cp "$1" "$tmp/fx.rs"
    CLIPPY_CONF_DIR="$conf" clippy-driver --edition 2024 --crate-type lib --emit=metadata \
        -o "$tmp/fx.rmeta" "$tmp/fx.rs" -A warnings \
        -D clippy::disallowed_methods -D clippy::disallowed_types 2>&1 \
      | grep -cE '^error: use of a disallowed (method|type)' || true
}
want=$(grep -c '// G1D' "$fx/positive.rs.txt")
got=$(lint "$fx/positive.rs.txt")
if [ "$got" -ne "$want" ]; then
    echo "★ G1d SELF-TEST FAILED: the known positive drew $got diagnostics, want $want"
    echo "  (an entry of $conf/clippy.toml no longer resolves, or the fixture drifted)."
    exit 1
fi
got=$(lint "$fx/perimeter.rs.txt")
if [ "$got" -ne 0 ]; then
    echo "★ G1d SELF-TEST FAILED: the perimeter's opt-out did not hold ($got diagnostics)"
    exit 1
fi
echo "G1d self-test: $want of $want known positives found; the perimeter opt-out holds"

pkgs=$(python3 - <<'PY'
import tomllib
ws = tomllib.load(open("Cargo.toml", "rb"))["workspace"]["members"]
names = []
for m in ws:
    n = tomllib.load(open(f"{m}/Cargo.toml", "rb"))["package"]["name"]
    if n.startswith("kf-"):
        names += ["-p", n]
print(" ".join(names))
PY
)
n=$(echo "$pkgs" | grep -o -- '-p ' | wc -l)
if [ "$n" -lt 18 ]; then
    echo "★ G1d names only $n kf-* crates (floor 18): the scan is blind"; exit 1
fi
echo "G1d: $n kf-* crates"
# shellcheck disable=SC2086
CLIPPY_CONF_DIR="$conf" cargo clippy --locked --all-targets --target-dir "${G1D_TARGET_DIR:-target/address-clippy}" $pkgs -- \
    -A warnings -D clippy::disallowed_methods -D clippy::disallowed_types
echo "G1d: no disallowed pointer function or type outside the perimeter"
