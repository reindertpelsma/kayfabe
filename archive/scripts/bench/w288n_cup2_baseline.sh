#!/usr/bin/env bash
# ★★★★★ w288n — cup2 BASELINE ON THE PASSTHROUGH BUILD. One boot, one number.
#
# ⊘⊘ THIS MEASURES NOTHING THIS RUNG BUILDS. The error-notifier-over-guest-pages work is NOT
#    in this binary. The owner asked for this run explicitly because **cup2 has never been run
#    on the passthrough build at all** — the `GP_GET 0→1` result everyone quotes was the RAW
#    CLIENT's own channel (w287, `--ce-client`), not `cup2`, and `w287_run.sh` says so in its
#    own header: *"nothing here bounds cup2 … cup2 is deliberately NOT run."*
#    ⇒ Read this as a BASELINE for the goal metric and as nothing else.
#
# ★ PRE-REGISTERED PREDICTION: `CUP2_RC=124`. The GR fault is unaddressed by this revision, so
#   a 124 here is the EXPECTED result and is not evidence against the notifier design. The run
#   is worth its cost because the number has never been taken on this arming, and "we assumed
#   124" and "we measured 124" are different facts.
#
# Arming is w287's, carried byte for byte, so this is comparable to the boots that produced the
# passthrough result. The ONLY difference from `w287_run.sh` is the POST_CAPTURE_HOOK: the cup2
# gdbspin hook instead of the R33 raw-client hook.
set -uo pipefail
PFX=${W288N_TAG_PREFIX:-w288n}
OUT=/workspace/${PFX}_run.log
exec >"$OUT" 2>&1
# ★ START marker and EXIT line, so "file exists but has no terminator" is detectable at all.
#   ⊘ 143 = the JOB was SIGTERMed (dead); 124 = the LAUNCHER's ssh/timeout expired while the
#   job ran on fine. Opposite meanings arriving as the same word.
finish() { echo "=== W288N EXIT rc=$1 at $(date -Is) ==="; exit "$1"; }
echo "=== W288N START $(date -Is) pid=$$ ==="

export PATH=/root/.cargo/bin:$PATH
# ⚠⚠ LANE-PRIVATE, and this is not tidiness: two lanes sharing a tree clobbered each other on
#    2026-08-13 and a stale binary silently ran.
REPO=${KAYFABE_REPO:-/workspace/kayfabe_w288n}
cd "$REPO" || finish 90
HEAD=$(git rev-parse HEAD)
echo "=== HEAD=$HEAD ==="
DIRT=$(git status --porcelain --untracked-files=no)
[ -z "$DIRT" ] || { echo "=== ★ TREE IS DIRTY ==="; echo "$DIRT"; finish 91; }

export CARGO_TARGET_DIR=/workspace/bench/cargo-target-w288n
export KAYFABE_SHIM_FEATURES=host-isolates

# ★★★ DELETE THE BINARY FIRST, so "no build ⇒ no file ⇒ no run" rather than a stale artefact.
rm -f /workspace/bench/qemu-build/qemu-system-x86_64

echo "=== BUILD START $(date -Is) ==="
scripts/build_qom_shim.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build
BRC=$?
echo "=== BUILD RC=$BRC $(date -Is) ==="
[ $BRC -eq 0 ] || finish 92
[ -x /workspace/bench/qemu-build/qemu-system-x86_64 ] || { echo "=== ★★★ NO BINARY ==="; finish 92; }
# ⚠ From the SAME invocation: disk is the failure that reads as a code error.
echo "=== BUILD ENOSPC/LLVM = $(grep -c 'No space left on device\|LLVM ERROR' "$OUT" 2>/dev/null) | df: $(df -h /workspace | tail -1) ==="

# ★★★ THE STAMP GATE, anchored to exactly 40 hex. The bench once served a binary built from a
#     revision nobody chose for WEEKS.
STAMP=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 2>/dev/null \
        | grep -oE 'kayfabe-rev:[0-9a-f]{40}' | head -1 | cut -d: -f2)
echo "=== STAMP=$STAMP HEAD=$HEAD ==="
[ "$STAMP" = "$HEAD" ] || { echo "=== ★★★ STAMP GATE FAIL: the binary is not this HEAD ==="; finish 93; }

export KAYFABE_ISOLATES=real
export KAYFABE_CE_EXECUTOR=host
export NVKVM_RAM_BACKEND=memfd
export KAYFABE_GUEST_RAM=memfd
export POST_CAPTURE_HOOK=$REPO/scripts/bench/cup2_hook_gdbspin.sh
export GQ_TIMEOUT=420
export BOOT_TIMEOUT=180
for f in scripts/bench/guest_spinprobe.c scripts/bench/cup2_hook_gdbspin.sh; do
  [ -f "$f" ] || { echo "=== ★★★ MISSING $f — the probe cannot run ==="; finish 96; }
done

# The carried arming — w287's, unchanged. Named so a mis-armed boot is not a data point that
# looks like one.
export KAYFABE_FB_JOIN=shared
export KAYFABE_GUEST_RING=ring
export KAYFABE_GUEST_PUSHBUF=pin
export KAYFABE_PT_WITNESS_EXEC=on
export KAYFABE_GUEST_SEMA=pin
export KAYFABE_GR_ROUTE=passthrough
export KAYFABE_GUEST_OPERAND=pin
unset KAYFABE_PT_SWEEP KAYFABE_RING_VIDMEM

TAG=${PFX}_guest
echo "=== BOOT $TAG START $(date -Is) ==="
timeout 1500 "$REPO/scripts/bench/boot_capture.sh" "$TAG"
echo "=== BOOT $TAG RC=$? $(date -Is) ==="

Q=/workspace/bench/run_${TAG}_qemu.log
P=/workspace/bench/run_${TAG}_probe.log
D=/workspace/bench/run_${TAG}_hostdmesg.log

# ⊘ pgrep -x qemu-system-x86, NEVER qemu-system-x86_64: /proc/PID/comm truncates to 15 chars,
#   so the _64 form can never match and any check built on it passes vacuously.
echo "--- post-boot liveness: pgrep -x qemu-system-x86 = [$(pgrep -x qemu-system-x86 | tr '\n' ' ')] ss2223 = [$(ss -tln 2>/dev/null | grep -c 2223)]"
echo "--- ARTEFACT SIZES: qemu=$(stat -c %s "$Q" 2>/dev/null || echo MISSING) probe=$(stat -c %s "$P" 2>/dev/null || echo MISSING) hostdmesg=$(stat -c %s "$D" 2>/dev/null || echo MISSING)"

# ★★ The six carried arms, asserted out of the device's own lines.
for pair in "FB-JOIN arm=shared" "GUEST-RING arm=ring" "GUEST-PUSHBUF arm=pin" \
            "GUEST-SEMA arm=pin" "GR-ROUTE arm=passthrough" "GUEST-OPERAND arm=pin"; do
  grep -q "kayfabe: $pair" "$Q" 2>/dev/null \
    && echo "    ★ CARRIED-ARM: PASS ($pair)" \
    || echo "    ★★★ CARRIED-ARM: FAIL — wanted '$pair'. ⊘ VOID for comparison."
done

echo "=== ★★★★★ THE GOAL METRIC ==="
# ★★★★★ CUP2_RC — ANCHORED. The anchor is the whole point.
# ⊘⊘ `grep -o 'CUP2_RC=[0-9]*'` ALSO matches `GCC_CUP2_RC=0`, the guest COMPILER's status, and
#    has reported CUP2_RC=0 — THE CAMPAIGN'S HEADLINE SUCCESS VALUE — on an arm that was hanging.
rc=$(grep -oE '(^|[^A-Z_])CUP2_RC=[0-9]+' "$P" 2>/dev/null | grep -oE 'CUP2_RC=[0-9]+' | tail -1)
echo "--- ★★★★★ CUP2_RC = [${rc:-⊘ NO cup2 EXIT LINE — THE MEASUREMENT DID NOT HAPPEN. ⊘ This is NOT 0 and must never be read as one}]"
echo "    (⊘ disambiguated from GCC_CUP2_RC, the compiler's status: $(grep -c 'GCC_CUP2_RC' "$P" 2>/dev/null) such line(s) present)"
echo "    unanchored, for contrast: [$(grep -oh 'CUP2_RC=[0-9]*' "$P" 2>/dev/null | tr '\n' ' ')]"
echo "--- cup2's own last prints:"
grep -E '^ok |^CUP2_RC|totalMem' "$P" 2>/dev/null | tail -12 | sed 's/^/      /'

echo "=== THE FAULT, from the HOST's own dmesg (identity, not a count) ==="
grep -E 'Xid' "$D" 2>/dev/null | tail -5 | sed 's/^/      /'
echo "      Xid lines = [$(grep -c 'Xid' "$D" 2>/dev/null)]"

echo "=== ERROR-NOTIFIER SURFACE (expected ABSENT at this revision — a known-negative) ==="
echo "      h_object_error census lines = [$(grep -c 'ERROR-NOTIFIER\|hObjectError' "$Q" 2>/dev/null)]"

finish 0
