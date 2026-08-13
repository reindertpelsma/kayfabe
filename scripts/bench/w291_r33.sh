#!/usr/bin/env bash
# ★★★★★ w291 STEP 1 — THE RAW CE CLIENT ON THE CURRENT BUILD, WITH AND WITHOUT RELAXATIONS.
#
#   $1 = relaxed | bare    (⊘ REQUIRED, never defaulted)
#     relaxed : KAYFABE_PT_SWEEP=on  KAYFABE_OPERAND_JOIN=join   — the known-passing baseline
#     bare    : both UNSET                                       — ★★★ THE SHIPPING QUESTION
#
# ⊘ `KAYFABE_VAS_PUBLISH=off` on BOTH arms, deliberately. Leg 8 is a THIRD relaxation and
#   arming it here would make "does arm 1 need the relaxations" unanswerable — the exact
#   accumulation the owner's narrowing forbids.
#
# ★★★ REPORT THE FOUR FACTS, NOT A WORD. `CeEvidence::met_the_whole_bar` is a conjunction of
#   FOUR (`rm.rs:3881-3907`): bytes moved / dst correct first AND last / engine semaphore ==
#   declared payload / GP_GET == GP_PUT. `[measured w283c]` a ★ line once printed "GP_GET 0
#   caught GP_PUT 1" and returned R33_RC=0, because the verdict implemented THREE of the four
#   and the word "caught" is template text. ⇒ this harness prints the client's own line
#   VERBATIM and never a pass/fail word of its own.
set -uo pipefail
ARM="${1:-}"
case "$ARM" in relaxed|bare) ;; *) echo "usage: $0 relaxed|bare" >&2; exit 64 ;; esac
TAG=w291r33${ARM}
OUT=/workspace/${TAG}.log
exec >"$OUT" 2>&1
finish() { echo "=== W291 EXIT rc=$1 arm=$ARM at $(date -Is) ==="; exit "$1"; }
echo "=== W291 START arm=$ARM $(date -Is) pid=$$ ==="
export PATH=/root/.cargo/bin:$PATH
REPO=/workspace/kayfabe_w290
cd "$REPO" || finish 90
HEAD=$(git rev-parse HEAD); echo "=== HEAD=$HEAD ==="
DIRT=$(git status --porcelain --untracked-files=no)
[ -z "$DIRT" ] || { echo "=== TREE IS DIRTY ==="; echo "$DIRT"; finish 91; }

export CARGO_TARGET_DIR=/workspace/bench/cargo-target-w290
export KAYFABE_SHIM_FEATURES=host-isolates
CLIENT=$CARGO_TARGET_DIR/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder
[ -x "$CLIENT" ] || finish 95
echo "=== CLIENT $(md5sum "$CLIENT" | cut -d' ' -f1) ==="

rm -f /workspace/bench/qemu-build/qemu-system-x86_64
echo "=== BUILD SHIM $(date -Is) ==="
scripts/build_qom_shim.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build
BRC=$?; echo "=== BUILD RC=$BRC $(date -Is) ==="
[ $BRC -eq 0 ] || finish 92
STAMP=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 2>/dev/null | grep -oE 'kayfabe-rev:[0-9a-f]{40}' | head -1 | cut -d: -f2)
echo "=== STAMP=$STAMP HEAD=$HEAD ==="
[ "$STAMP" = "$HEAD" ] || { echo "=== ★★★ STAMP GATE FAIL ==="; finish 93; }

export KAYFABE_ISOLATES=real
export KAYFABE_CE_EXECUTOR=host
export NVKVM_RAM_BACKEND=memfd
export KAYFABE_GUEST_RAM=memfd
export GQ_TIMEOUT=420
export BOOT_TIMEOUT=180
# the carried, NON-relaxation arming — identical on both arms
export KAYFABE_FB_JOIN=shared
export KAYFABE_GUEST_RING=ring
export KAYFABE_GUEST_PUSHBUF=pin
export KAYFABE_PT_WITNESS_EXEC=on
export KAYFABE_GUEST_SEMA=pin
export KAYFABE_GR_ROUTE=passthrough
export KAYFABE_GUEST_OPERAND=pin
export KAYFABE_VAS_PUBLISH=off      # ⊘ leg 8 OFF on both arms — see the header
unset KAYFABE_RING_VIDMEM
if [ "$ARM" = relaxed ]; then
  export KAYFABE_PT_SWEEP=on        # ⊘ RELAXATION 1
  export KAYFABE_OPERAND_JOIN=join  # ⊘ RELAXATION 2
else
  unset KAYFABE_PT_SWEEP KAYFABE_OPERAND_JOIN
fi

export POST_CAPTURE_HOOK=$REPO/scripts/bench/r33_hook_ce_client.sh
export KAYFABE_R33_BIN=$CLIENT
export KAYFABE_R33_ARGS="--ce-client-fault"
echo "=== BOOT $TAG START $(date -Is) ARGS=[$KAYFABE_R33_ARGS] ==="
timeout 1800 "$REPO/scripts/bench/boot_capture.sh" "$TAG"
echo "=== BOOT $TAG RC=$? $(date -Is) ==="
Q=/workspace/bench/run_${TAG}_qemu.log
P=/workspace/bench/run_${TAG}_probe.log
D=/workspace/bench/run_${TAG}_hostdmesg.log

echo ""
echo "=== ★ THE ARMING ACTUALLY IN FORCE (a boot happening is not an arm running) ==="
echo "    PT_SWEEP env     = [${KAYFABE_PT_SWEEP:-<unset>}]"
echo "    OPERAND_JOIN env = [${KAYFABE_OPERAND_JOIN:-<unset>}]"
grep -oE 'OPERAND-JOIN arm=[a-z]+' "$Q" 2>/dev/null | head -1 | sed 's/^/      observed: /'
grep -oE 'VAS-PUBLISH arm=[a-z]+' "$Q" 2>/dev/null | head -1 | sed 's/^/      observed: /'
echo "      PT-SWEEP lines = [$(grep -c 'PT-SWEEP tasks=' "$Q" 2>/dev/null)]  (⊘ on the bare arm this SHOULD be 0)"
echo "      VAS-PUBLISH lines = [$(grep -c 'VAS-PUBLISH token=' "$Q" 2>/dev/null)]  (MUST be 0 on both arms)"

echo ""
echo "=== ⊘ DID THE CLIENT RUN AT ALL? Every zero below is VACUOUS if it did not ==="
echo "    R33 lines in the probe log = [$(grep -c ' R33 ' "$P" 2>/dev/null)]"
echo "    'probe could not be built'  = [$(grep -c 'the probe could not be built' "$P" 2>/dev/null)]  (MUST be 0)"

echo ""
echo "=== ★★★★★ THE CLIENT'S OWN OUTPUT — EVERY R33 LINE, VERBATIM, ARMS SEPARATE ==="
grep -E '^(★|FAIL|\?\?|ok|info|⊘) +R33 ' "$P" 2>/dev/null | sed 's/^/    /'

echo ""
echo "=== ★★★★★ ARM 1, THE FOUR FACTS OF met_the_whole_bar() — NOT A PASS/FAIL WORD ==="
echo "⊘ The conjunction is FOUR (rm.rs:3881-3907). Read the NUMBERS; 'caught' is template text."
A1=$(grep -E ' R33 arm 1 COPY ' "$P" 2>/dev/null | tail -1)
echo "    verbatim: [${A1:-⊘ NO arm-1 LINE — THE MEASUREMENT DID NOT HAPPEN. ⊘ This is NOT a fail}]"
echo "    fact 1 bytes moved      = [$(echo "$A1" | grep -oE '[0-9]+ bytes moved')]"
echo "    fact 2 dst first/last   = [$(echo "$A1" | grep -oE 'dst\[0\][^,]*')] [$(echo "$A1" | grep -oE 'dst\[last\][^,]*')]"
echo "    fact 3 semaphore/declared = [$(echo "$A1" | grep -oE '(engine )?semaphore [^,]*')]"
echo "    fact 4 GP_GET vs GP_PUT = [$(echo "$A1" | grep -oE 'GP_GET [0-9]+ (caught )?GP_PUT [0-9]+')]"
echo "    ⊘ which line kind       = [$(echo "$A1" | grep -oE '^(★|FAIL)')]  (★ = all four; FAIL = the line names WHICH failed)"
echo "    ⊘ the named diagnosis   = [$(echo "$A1" | grep -oE 'THREE OF FOUR[^\"]*' | cut -c1-160)]"

echo ""
echo "=== ★★★ ARMS 4 AND 6, STATED SEPARATELY — 'arm 1 passes' is NOT 'the client passes' ==="
for a in 2 3 4 5 6; do
  echo "    arm $a: [$(grep -E " R33 arm $a " "$P" 2>/dev/null | tail -1 | cut -c1-190)]"
done

echo ""
echo "=== ★★★★★ R33_RC — ANCHORED, WITH THE UNANCHORED CONTRAST ==="
rc=$(grep -oE '(^|[^A-Z_])R33_RC=[0-9]+' "$P" 2>/dev/null | grep -oE 'R33_RC=[0-9]+' | tail -1)
echo "--- ★★★★★ R33_RC = [${rc:-⊘ NO R33 EXIT LINE — THE MEASUREMENT DID NOT HAPPEN. ⊘ This is NOT 0}]"
echo "    unanchored, for contrast: [$(grep -oh 'R33_RC=[0-9]*' "$P" 2>/dev/null | tr '\n' ' ')]"

echo ""
echo "=== ★ HOST-PUBLISHED / host_rows — the build's own state on this arm ==="
grep -oE 'host_rows=[0-9]+ of [0-9]+' "$Q" 2>/dev/null | sort -u | head -10 | sed 's/^/      /'
echo "      (⊘ leg 8 is OFF here; any host_rows>0 is leg 7's operand join, not leg 8)"

echo ""
echo "=== ★ THE FAULT, BY IDENTITY (host dmesg, watermarked to THIS boot) ==="
grep -E 'Xid' "$D" 2>/dev/null | sed 's/^/      /'
echo "      Xid count = [$(grep -c Xid "$D" 2>/dev/null)]"

echo ""
echo "=== ★★ HARNESS SELF-CHECK — assert this block's own output exists ==="
echo "    lines written so far = [$(wc -l < "$OUT")]"
echo "    qemu log bytes       = [$(stat -c%s "$Q" 2>/dev/null || echo MISSING)]"
echo "    probe log bytes      = [$(stat -c%s "$P" 2>/dev/null || echo MISSING)]"

finish 0
