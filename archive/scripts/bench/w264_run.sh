#!/usr/bin/env bash
# w264 — leg 4: the guest-RAM PIN's second source, the pushbuffer VAs the ring's own
# GPFIFO entries name. Build at HEAD, assert the stamp, then FOUR boots.
#
# ★★★ THE LADDER — every arm differs from the one above it in EXACTLY ONE VARIABLE.
#
#   arm    KAYFABE_FB_JOIN  KAYFABE_GUEST_RING  KAYFABE_GUEST_PUSHBUF
#   base   (unset)          (unset)             (unset)     <= w262's control SHAPE
#   join   shared           (unset)             (unset)     <= w263's `off` arm
#   ring   shared           ring                (unset)     <= w263's `ring` arm
#   pin    shared           ring                pin         <= THE RUNG
#
# ⊘⊘ WHY `base` EXISTS, and it is the defect w263's own RESULT §3.1 reported against itself:
# `w263_run.sh` exported `KAYFABE_FB_JOIN=shared` UNCONDITIONALLY, so its `off` arm had the
# supply side armed and was NOT w262's control — yet six of its scorecard rows were compared
# to w262's numbers anyway, across boots and across revisions. ⇒ Rather than compare across
# boots, this run puts w262's shape INSIDE the ladder, at one revision, in one campaign. A
# comparison that needs two boot campaigns to be meaningful is a comparison nobody can audit.
#
# ⚠ Cost: one extra boot (~15 min). That is the price of the six rows being readable.
#
# ★ START marker and EXIT line so "file exists but has no terminator" is detectable at all
#   (the SIGTERM/zero-byte trap: 143 = the JOB died, 124 = the LAUNCHER's ssh expired while
#   the job kept running — opposite meanings arriving as the same word).
OUT=/workspace/w264_run.log
exec >"$OUT" 2>&1
finish() { echo "=== W264 EXIT rc=$1 at $(date -Is) ==="; exit "$1"; }
echo "=== W264 START $(date -Is) pid=$$ ==="

export PATH=/root/.cargo/bin:$PATH
REPO=/workspace/kayfabe_w264
cd "$REPO" || finish 90
HEAD=$(git rev-parse HEAD)
echo "=== HEAD=$HEAD ==="
DIRT=$(git status --porcelain --untracked-files=no)
[ -z "$DIRT" ] || { echo "=== ★ TREE IS DIRTY ==="; echo "$DIRT"; finish 91; }

export CARGO_TARGET_DIR=/workspace/bench/cargo-target-w264
export KAYFABE_SHIM_FEATURES=host-isolates
echo "=== BUILD START $(date -Is) ==="
scripts/build_qom_shim.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build
BRC=$?
echo "=== BUILD RC=$BRC $(date -Is) ==="
[ $BRC -eq 0 ] || finish 92

# ★★★ THE STAMP GATE. The bench silently served a binary built from `862c7c2` for weeks;
#     every claim attributed to HEAD was not HEAD's. A boot is refused unless the binary
#     carries the revision this tree is at.
STAMP=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 | grep -o 'kayfabe-rev:[0-9a-f]*\(-dirty\)\?' | sort -u)
echo "=== STAMP: [$STAMP] WANT: [kayfabe-rev:$HEAD] ==="
if [ "$STAMP" != "kayfabe-rev:$HEAD" ]; then
  echo "=== ★★★ STAMP MISMATCH — REFUSING TO BOOT. ==="
  finish 93
fi
echo "=== STAMP GATE: PASS ==="
echo "kayfabe-rev:$HEAD" > /workspace/bench/BUILD_REV.txt

# ★ CONTENT CHECK: the changed code is actually IN this binary. A stamp says which
#   REVISION; these say which CODE. ⊘ A zero for any of them means the feature was compiled
#   out (host-isolates) and the boot would measure a build nobody wrote.
#   ⚠ Asserted, not merely printed — a check whose failure is a line in a log nobody greps
#   is not a check.
echo "=== CONTENT CHECK ==="
CC_RC=0
for s in "PB-PIN" "GUEST-PUSHBUF" "KAYFABE_GUEST_PUSHBUF" "CAPPED" "NOT ONE PAGE RESOLVED IN GUEST RAM" "pb run"; do
  n=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 | grep -c -- "$s")
  printf '  %-38s = %s\n' "$s" "$n"
  [ "$n" -gt 0 ] || { echo "  ★★★ ZERO — the changed code is NOT in this binary"; CC_RC=1; }
done
[ $CC_RC -eq 0 ] || finish 94

export KAYFABE_ISOLATES=real
export KAYFABE_CE_EXECUTOR=host
export NVKVM_RAM_BACKEND=memfd
export KAYFABE_GUEST_RAM=memfd
export POST_CAPTURE_HOOK=$REPO/scripts/bench/cup2_hook_w232.sh
export GQ_TIMEOUT=300
export BOOT_TIMEOUT=180

# ⊘ EVERY per-arm variable is unset first and set only if this arm names it, so an arm can
#   never inherit the previous arm's arming. That inheritance is precisely what made w263's
#   control not its predecessor's.
boot() {
  local tag=$1 fbj=$2 ring=$3 pb=$4
  unset KAYFABE_FB_JOIN KAYFABE_GUEST_RING KAYFABE_GUEST_PUSHBUF
  [ "$fbj"  = "-" ] || export KAYFABE_FB_JOIN=$fbj
  [ "$ring" = "-" ] || export KAYFABE_GUEST_RING=$ring
  [ "$pb"   = "-" ] || export KAYFABE_GUEST_PUSHBUF=$pb
  echo "=== BOOT $tag START $(date -Is) ==="
  echo "    KAYFABE_FB_JOIN=[${KAYFABE_FB_JOIN:-unset}]" \
       "KAYFABE_GUEST_RING=[${KAYFABE_GUEST_RING:-unset}]" \
       "KAYFABE_GUEST_PUSHBUF=[${KAYFABE_GUEST_PUSHBUF:-unset}]"
  timeout 900 "$REPO/scripts/bench/boot_capture.sh" "$tag"
  echo "=== BOOT $tag RC=$? $(date -Is) ==="
  # ⊘ pgrep -x qemu-system-x86 (comm truncates at 15); NOT -f (it matches the asker).
  echo "--- post-boot liveness: pgrep -x qemu-system-x86 = [$(pgrep -x qemu-system-x86 | tr '\n' ' ')] ss2223 = [$(ss -tln 2>/dev/null | grep -c 2223)]"
  # ⚠ From the SAME invocation that produced the status, per CLAUDE.md.
  echo "--- ENOSPC_LLVM=$(grep -c 'No space left on device\|LLVM ERROR' /workspace/bench/run_${tag}_qemu.log 2>/dev/null || echo '?')"
  # ★ The arming, read back OUT OF THE BOOT'S OWN LOG rather than out of this shell. A
  #   variable this script exported and a variable the device read are two different facts.
  echo "--- ARMING AS THE DEVICE SAW IT:"
  grep -m1 -E 'kayfabe: (FB-JOIN|GUEST-RING|GUEST-PUSHBUF) arm=' /workspace/bench/run_${tag}_qemu.log 2>/dev/null
  grep -m1 'kayfabe: GUEST-RING arm='   /workspace/bench/run_${tag}_qemu.log 2>/dev/null
  grep -m1 'kayfabe: GUEST-PUSHBUF arm=' /workspace/bench/run_${tag}_qemu.log 2>/dev/null
}

#     tag         FB_JOIN   GUEST_RING  GUEST_PUSHBUF
boot w264_base    -         -           -
boot w264_join    shared    -           -
boot w264_ring    shared    ring        -
boot w264_pin     shared    ring        pin

echo "=== ARTEFACT SIZES ==="
ls -l /workspace/bench/run_w264_* 2>/dev/null
finish 0
