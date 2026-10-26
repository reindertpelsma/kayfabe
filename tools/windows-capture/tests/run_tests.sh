#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Everything that can be tested without Windows, a GPU or a VM.
#
#   tests/run_tests.sh
#
# Optional environment:
#   KFWC_KIT=DIR         extracted Windows kit (see build/build-linux.sh): also builds the driver, runs the
#                        PE check, the import guard and the negative compile test
#   KFWC_CENSUS_BIN=PATH a built kf-census (cargo build -p kf-census): runs the real decoder on our output
#   OUT=DIR              scratch (default /tmp/kfwc-build); the Rust fixture builds under /tmp/kfwc-target-fx
#                        and is deleted afterwards (about 40 MB)
# Exit status 0 only if every step passed.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
repo=$(cd "$root/../.." && pwd)
OUT=${OUT:-/tmp/kfwc-build}
fail=0
step() { printf '\n== %s\n' "$*"; }
run() { "$@" || { echo "FAILED: $*"; fail=$((fail + 1)); }; }

step "unit tests, synthetic queues (ASan + UBSan)"
run make -C "$here" OUT="$OUT" test
step "ring: two-thread stress (ASan/UBSan, then ThreadSanitizer)"
run make -C "$here" OUT="$OUT" "$OUT/test_ring" "$OUT/test_ring_tsan"
run "$OUT/test_ring"
run "$OUT/test_ring_tsan"
step "sidecar / metadata"
run make -C "$here" OUT="$OUT" "$OUT/test_meta"
run "$OUT/test_meta"
step "collector header visible before records arrive"
run make -C "$here" OUT="$OUT" "$OUT/test_file"
run "$OUT/test_file"
step "family/revisit mutation check (two distinct killers per mutant)"
run python3 -I "$here/mutate_family.py" "$OUT/family-mutants"
step "native rehearsal integrity and transport"
run python3 -I "$here/test_verify_pull.py"
run python3 -I "$here/test_rehearse_transport.py"
step "read-only guard + its self-test"
run python3 -I "$here/guard_readonly.py"
run python3 -I "$here/guard_readonly.py" --self-test
step "mutation check (the tests must fail when the library is broken)"
run python3 -I "$here/mutate.py" --out "$OUT/mut"

step "round trip: kayfabe's own encoder -> rings -> engine -> KFGSPCAP"
fx=${KFWC_FIXTURE:-/tmp/kfwc-fixture.bin}
fx_target=${KFWC_FIXTURE_TARGET:-/tmp/kfwc-target-fx}
(
    cd "$here/kf-fixture"
    CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_TARGET_DIR="$fx_target" \
        nice -n 19 cargo run --offline --release -j2 -q -- "$fx" 2>&1 \
        | grep -E "^wrote|^error"
) || { echo "fixture build failed"; fail=$((fail + 1)); }
rm -rf "$fx_target"
run make -C "$here" OUT="$OUT" "$OUT/test_kayfabe_roundtrip"
run "$OUT/test_kayfabe_roundtrip" "$fx" "$OUT/rt"
step "observer comparison script (validation step 2) against synthetic observer exports"
run python3 -I "$here/test_compare.py" "$OUT/rt.pre610.kfgspcap" > /dev/null

if [[ -n ${KFWC_CENSUS_BIN:-} ]]; then
    step "the real decoder (kf-census) on the module's output"
    for pair in "580.82.07 pre610" "610.43.02 610"; do
        set -- $pair
        "$KFWC_CENSUS_BIN" --ogkm-tag "$1" --out "$OUT/census-$2" "$OUT/rt.$2.kfgspcap" | head -3
        [[ ${PIPESTATUS[0]} -eq 0 ]] || { echo "FAILED: decoder rejected $2"; fail=$((fail + 1)); }
    done
    # lossy run: the decoder MUST complain, and its missing-message count must equal the engine's (87)
    "$KFWC_CENSUS_BIN" --ogkm-tag 580.82.07 --out "$OUT/census-lapped" "$OUT/rt.lapped.kfgspcap" > "$OUT/lapped.txt"
    st=$?
    if [[ $st -ne 1 ]] || ! grep -q "capture_loss" "$OUT/lapped.txt" || ! grep -q "missing messages (sum of forward gaps): 87" "$OUT/lapped.txt"; then
        echo "FAILED: lossy capture not reported as expected"; cat "$OUT/lapped.txt"; fail=$((fail + 1))
    else echo "lossy capture: announced loss and 87 missing messages, as counted by the engine"; fi
else
    echo "(KFWC_CENSUS_BIN not set: decoder step skipped)"
fi

if [[ -n ${KFWC_KIT:-} ]]; then
    step "driver + companion build, PE check, import guard"
    run "$root/build/build-linux.sh" "$OUT/win"
    step "negative compile: a store through the mapped pointer must not compile"
    inj=$OUT/neg; rm -rf "$inj"; mkdir -p "$inj"
    cp -r "$root/driver" "$root/lib" "$inj/"
    python3 -I - "$inj/driver/kfwc_drv.c" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read()
s = s.replace("    *out = *(const volatile ULONG *)(s->bar0 + off);", "    *out = *(const volatile ULONG *)(s->bar0 + off);\n    s->bar0[off] = 1;")
open(p, "w").write(s)
PY
    kit=10.0.28000.0
    if clang-cl /nologo /c /kernel /W4 /WX /O2 /GS /D_AMD64_ /DAMD64 /D_WIN64 /D_WIN32_WINNT=0x0A00 /DNTDDI_VERSION=0x0A000008 /D_KERNEL_MODE /clang:-Wcast-qual \
        "/imsvc$KFWC_KIT/wdk/c/Include/$kit/km" "/imsvc$KFWC_KIT/wdk/c/Include/$kit/km/crt" "/imsvc$KFWC_KIT/sdk/c/Include/$kit/shared" "/imsvc$KFWC_KIT/sdk/c/Include/$kit/ucrt" \
        "/Fo$inj/x.obj" "$inj/driver/kfwc_drv.c" > "$inj/log.txt" 2>&1; then
        echo "FAILED: the injected store through KFWC_MMIO compiled"; fail=$((fail + 1))
    else grep -m1 "read-only\|const" "$inj/log.txt" | sed 's/^/compiler said: /'; fi
else
    echo "(KFWC_KIT not set: driver build steps skipped)"
fi
if [[ -n ${KFWC_PWSH:-} ]]; then
    step "PowerShell operator helpers"
    run "$KFWC_PWSH" -NoProfile -File "$here/test_driver_key.ps1" \
        -Helper "$root/rehearse/driver_key.ps1"
    run "$KFWC_PWSH" -NoProfile -File "$here/test_capture_dir.ps1" \
        -Helper "$root/rehearse/capture_dir.ps1"
fi
step "summary"
if [[ $fail -eq 0 ]]; then echo "ALL STEPS PASSED"; else echo "$fail STEP(S) FAILED"; fi
exit $((fail > 0))
