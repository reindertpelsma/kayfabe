#!/usr/bin/env bash
# ★★★★★ w289 GUEST — CRITERION 1: does the GUEST observe THE SAME FAULT, BY IDENTITY?
#
# One boot. The raw CE client runs INSIDE the guest with --ce-client-fault, on the SAME
# binary the native arms just used. The join is host dmesg (this boot only) against the
# clients OWN in-process reads.
set -uo pipefail
OUT=/workspace/w289_join.log
exec >"$OUT" 2>&1
finish() { echo "=== W289 JOIN EXIT rc=$1 at $(date -Is) ==="; exit "$1"; }
echo "=== W289 JOIN START $(date -Is) pid=$$ ==="
export PATH=/root/.cargo/bin:$PATH
REPO=/workspace/kayfabe_w289
cd "$REPO" || finish 90
HEAD=$(git rev-parse HEAD); echo "=== HEAD=$HEAD ==="
DIRT=$(git status --porcelain --untracked-files=no)
[ -z "$DIRT" ] || { echo "=== TREE IS DIRTY ==="; echo "$DIRT"; finish 91; }

export CARGO_TARGET_DIR=/workspace/bench/cargo-target-w289
export KAYFABE_SHIM_FEATURES=host-isolates
CLIENT=$CARGO_TARGET_DIR/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder
[ -x "$CLIENT" ] || finish 95
echo "=== CLIENT $(md5sum "$CLIENT" | cut -d" " -f1) (⊘ must equal the NATIVE arms binary) ==="

# ★★★ DELETE THE SHIM FIRST: no build => no file => no run.
rm -f /workspace/bench/qemu-build/qemu-system-x86_64
echo "=== BUILD SHIM $(date -Is) ==="
scripts/build_qom_shim.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build
BRC=$?; echo "=== BUILD RC=$BRC $(date -Is) ==="
[ $BRC -eq 0 ] || finish 92
STAMP=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 2>/dev/null | grep -oE "kayfabe-rev:[0-9a-f]{40}" | head -1 | cut -d: -f2)
echo "=== STAMP=$STAMP HEAD=$HEAD ==="
[ "$STAMP" = "$HEAD" ] || { echo "=== ★★★ STAMP GATE FAIL ==="; finish 93; }

export KAYFABE_ISOLATES=real
export KAYFABE_CE_EXECUTOR=host
export NVKVM_RAM_BACKEND=memfd
export KAYFABE_GUEST_RAM=memfd
export GQ_TIMEOUT=420
export BOOT_TIMEOUT=180
# the carried arming — w287s / w288ns, byte for byte
export KAYFABE_FB_JOIN=shared
export KAYFABE_GUEST_RING=ring
export KAYFABE_GUEST_PUSHBUF=pin
export KAYFABE_PT_WITNESS_EXEC=on
export KAYFABE_GUEST_SEMA=pin
export KAYFABE_GR_ROUTE=passthrough
export KAYFABE_GUEST_OPERAND=pin
export KAYFABE_PT_SWEEP=on
export KAYFABE_OPERAND_JOIN=join
unset KAYFABE_RING_VIDMEM

export POST_CAPTURE_HOOK=$REPO/scripts/bench/r33_hook_ce_client.sh
export KAYFABE_R33_BIN=$CLIENT
export KAYFABE_R33_ARGS="--ce-client-fault"
TAG=${KAYFABE_TAG:-w289j}
echo "=== BOOT $TAG START $(date -Is)  ARGS=[$KAYFABE_R33_ARGS] ==="
timeout 1500 "$REPO/scripts/bench/boot_capture.sh" "$TAG"
echo "=== BOOT $TAG RC=$? $(date -Is) ==="
Q=/workspace/bench/run_${TAG}_qemu.log
P=/workspace/bench/run_${TAG}_probe.log
D=/workspace/bench/run_${TAG}_hostdmesg.log

for pair in "FB-JOIN arm=shared" "GUEST-RING arm=ring" "GUEST-PUSHBUF arm=pin" \
            "GUEST-SEMA arm=pin" "GR-ROUTE arm=passthrough" "GUEST-OPERAND arm=pin"; do
  grep -q "kayfabe: $pair" "$Q" 2>/dev/null && echo "    ★ CARRIED-ARM: PASS ($pair)" \
    || echo "    ★★★ CARRIED-ARM: FAIL — wanted [$pair]. ⊘ VOID for comparison."
done

echo ""
echo "=== ★★★★★ THE GUEST CLIENTS OWN OUTPUT — every R33 line, verbatim ==="
grep -E "^(★|FAIL|\?\?|ok|info|⊘) +R33 " "$P" 2>/dev/null | sed "s/^/    /"
echo "--- WHICH ARM DID IT RUN (the harness word is not enough — a boot happening is not an arm running):"
grep -E "NOTIFIER APERTURE|arm 4 +=|probe could not be built" "$P" 2>/dev/null | sed "s/^/    /"
echo "--- R33_RC, ANCHORED vs unanchored (the anchor trap fired on two consecutive rungs):"
echo "    anchored   = [$(grep -oE "^R33_RC=[0-9]+" "$P" 2>/dev/null | tail -1)]"
echo "    unanchored = [$(grep -oh "R33_RC=[0-9]*" "$P" 2>/dev/null | tr "\n" " ")]"
echo "--- the ioctl census (⚠ failed=0 is NOT nothing refused — RM puts status INSIDE the struct):"
grep -E "^  total=[0-9]+ failed=" "$P" 2>/dev/null | sed "s/^/    /"
echo "--- every ioctl the census marked with an errno (this is what named nr 39 last time):"
grep -E "^ +[0-9]+: nr .* errno " "$P" 2>/dev/null | sed "s/^/    /"
echo "--- our own device: did the error-notifier path fire at all?"
echo "    ERROR-NOTIFIER built   = [$(grep -c "ERROR-NOTIFIER .*memory=" "$Q" 2>/dev/null)]"
echo "    ERROR-NOTIFIER REFUSED = [$(grep -c "ERROR-NOTIFIER REFUSED" "$Q" 2>/dev/null)]"
grep -E "ERROR-NOTIFIER" "$Q" 2>/dev/null | head -20 | sed "s/^/      /"
echo "    MMU-FAULT-INFO relay lines:"
grep -iE "MMU-FAULT-INFO|GET_MMU_FAULT_INFO|fault_info" "$Q" 2>/dev/null | head -20 | sed "s/^/      /"

echo ""
echo "=== ★★★★★ CRITERION 1 — THE JOIN, FIELD BY FIELD, HOST vs GUEST, ONE RUN ==="
echo "⊘ HOST dmesg for THIS BOOT ONLY (boot_capture watermarks it):"
grep -E "Xid" "$D" 2>/dev/null | sed "s/^/    HOST: /"
H=$(grep -E "Xid" "$D" 2>/dev/null | tail -1)
BUILT=$(grep -c "the probe could not be built" "$P" 2>/dev/null)
echo ""
echo "    ⊘ PROBE-COULD-NOT-BE-BUILT lines = [$BUILT] — if this is NOT 0, EVERY zero below is"
echo "      VACUOUS: it counts a guest-side read that never happened."
echo "    host Xid code   = [$(echo "$H" | grep -oE "\): [0-9]+," | grep -oE "[0-9]+")]"
echo "    host engine     = [$(echo "$H" | grep -oE "ENGINE [A-Z0-9_]+")]"
echo "    host hubclient  = [$(echo "$H" | grep -oE "HUBCLIENT_[A-Z0-9_]+")]"
echo "    host address    = [$(echo "$H" | grep -oE "faulted @ 0x[0-9a-f_]+")]"
echo "    host fault type = [$(echo "$H" | grep -oE "type FAULT_[A-Z_]+")]"
echo "    host access     = [$(echo "$H" | grep -oE "ACCESS_TYPE_[A-Z_]+")]"
echo "    GUEST Xid code  = [$(grep -oE "info32 0x[0-9a-f]+" "$P" 2>/dev/null | tail -1)]   (0x1f = 31)"
echo "    GUEST engine    = [$(grep -oE "info16 engine 0x[0-9a-f]+" "$P" 2>/dev/null | tail -1)]  ⚠ engine TYPE, not instance"
echo "    GUEST status    = [$(grep -oE "status 0x[0-9a-f]+" "$P" 2>/dev/null | tail -1)]"
echo "    GUEST faultType = [$(grep -oE "faultString=\"[A-Z_]+\"" "$P" 2>/dev/null | tail -1)]"
echo "    GUEST asked     = [$(grep -oE "asked 0x[0-9a-f]+" "$P" 2>/dev/null | tail -1)]"
echo "    GUEST reported  = [$(grep -oE "reported 0x[0-9a-f]+" "$P" 2>/dev/null | tail -1)]"
echo "    ★★ VA-IDENTITY HOLDS lines  = [$(grep -c "VA-IDENTITY HOLDS" "$P" 2>/dev/null)]"
echo "    ⊘⊘ VA-IDENTITY BROKEN lines = [$(grep -c "VA-IDENTITY BROKEN" "$P" 2>/dev/null)]  (MUST be 0)"
echo "    ⊘ PLANE D UNMEASURED        = [$(grep -c "PLANE D UNMEASURED" "$P" 2>/dev/null)]"
echo "    ⊘ PLANE A QUIET             = [$(grep -c "PLANE A QUIET" "$P" 2>/dev/null)]"
echo ""
echo "=== ★★ THE OTHER JOIN — arm 1s operands vs any host Xid on this boot ==="
echo "⊘ w288nc1 had a host Xid @ 0x1_20000000 it could not attribute. R33 now prints the"
echo "  operand addresses, so that attribution is a string comparison rather than a guess."
grep -E "R33 arm 1 OPERANDS" "$P" 2>/dev/null | sed "s/^/    /"
grep -E "R33 ADDRESS SPACES" "$P" 2>/dev/null | sed "s/^/    /"
echo ""
echo "=== the guest drivers own word ==="
grep -iE "nvrm|xid" /workspace/bench/run_${TAG}_dmesg.log 2>/dev/null | tail -12 | sed "s/^/    /"
finish 0
