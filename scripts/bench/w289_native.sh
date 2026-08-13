#!/usr/bin/env bash
# ★★★★★ w289 NATIVE — does the SYSMEM error notifier build AT ALL, on real hardware?
#
# ⊘ Two arms of the SAME binary, minutes apart, on one GA106. The only thing that differs is
#   the notifier aperture. The VIDMEM arm is w287s carried known-positive and must still
#   fire; the SYSMEM arm is the one `w288nc1` could not construct.
set -uo pipefail
OUT=/workspace/w289_native.log
exec >"$OUT" 2>&1
echo "=== W289 NATIVE START $(date -Is) pid=$$ ==="
export PATH=/root/.cargo/bin:$PATH
REPO=/workspace/kayfabe_w289
cd "$REPO" || exit 90
echo "=== HEAD=$(git rev-parse HEAD) DIRT=[$(git status --porcelain --untracked-files=no)] ==="
export CARGO_TARGET_DIR=/workspace/bench/cargo-target-w289
CLIENT=$CARGO_TARGET_DIR/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder
[ -x "$CLIENT" ] || { echo "=== NO CLIENT BINARY — no build => no file => no run ==="; echo "=== W289 NATIVE EXIT rc=95 ==="; exit 95; }
echo "=== BINARY $(md5sum "$CLIENT") ==="
nvidia-smi --query-gpu=name,driver_version --format=csv,noheader

run_arm () {
  local tag=$1; shift
  echo ""
  echo "############ ARM $tag : $* ############"
  local x0 x1
  x0=$(dmesg 2>/dev/null | grep -c Xid)
  timeout 300 "$CLIENT" "$@" > /tmp/w289_$tag.out 2>&1
  echo "ARM_${tag}_RC=$?"
  x1=$(dmesg 2>/dev/null | grep -c Xid)
  echo "--- the client output, R33 lines only (identities, not tallies):"
  grep -E "^(★|FAIL|\?\?|ok|info|⊘) +R33 " /tmp/w289_$tag.out | sed "s/^/    /"
  echo "--- WHICH ARM DID IT ACTUALLY RUN (the harness word is not enough):"
  grep -E "NOTIFIER APERTURE|arm 4 +=" /tmp/w289_$tag.out | sed "s/^/    /"
  echo "--- the ioctl census (⚠ failed=0 is NOT nothing refused):"
  grep -E "^  total=[0-9]+ failed=" /tmp/w289_$tag.out | sed "s/^/    /"
  echo "--- any ioctl the census marked with an errno:"
  grep -E "^ +[0-9]+: nr .* errno " /tmp/w289_$tag.out | sed "s/^/    /"
  echo "--- HOST Xid delta: $x0 -> $x1"
  dmesg 2>/dev/null | grep Xid | tail -2 | sed "s/^/    HOST: /"
  echo "--- ★★★★★ THE JOIN, field by field (host dmesg vs the clients OWN process):"
  local H
  H=$(dmesg 2>/dev/null | grep Xid | tail -1)
  echo "    host Xid code   = [$(echo "$H" | grep -oE "\): [0-9]+," | grep -oE "[0-9]+")]"
  echo "    host engine     = [$(echo "$H" | grep -oE "ENGINE [A-Z0-9_]+")]"
  echo "    host address    = [$(echo "$H" | grep -oE "faulted @ 0x[0-9a-f_]+")]"
  echo "    host fault type = [$(echo "$H" | grep -oE "type FAULT_[A-Z_]+")]"
  echo "    host access     = [$(echo "$H" | grep -oE "ACCESS_TYPE_[A-Z_]+")]"
  echo "    guest info32    = [$(grep -oE "info32 0x[0-9a-f]+" /tmp/w289_$tag.out | tail -1)]"
  echo "    guest engine    = [$(grep -oE "info16 engine 0x[0-9a-f]+" /tmp/w289_$tag.out | tail -1)]"
  echo "    guest asked     = [$(grep -oE "asked 0x[0-9a-f]+" /tmp/w289_$tag.out | tail -1)]"
  echo "    guest reported  = [$(grep -oE "reported 0x[0-9a-f]+" /tmp/w289_$tag.out | tail -1)]"
  echo "    VA-IDENTITY BROKEN lines = [$(grep -c "VA-IDENTITY BROKEN" /tmp/w289_$tag.out)]  (MUST be 0)"
  echo "    VA-IDENTITY HOLDS  lines = [$(grep -c "VA-IDENTITY HOLDS" /tmp/w289_$tag.out)]"
  echo "    PLANE D UNMEASURED       = [$(grep -c "PLANE D UNMEASURED" /tmp/w289_$tag.out)]"
  echo "    ⊘ a zero on the three counters above is VACUOUS unless the probe was BUILT:"
  echo "      probe-could-not-be-built lines = [$(grep -c "the probe could not be built" /tmp/w289_$tag.out)]"
  echo "--- ANCHORED vs unanchored R33_RC (the anchor trap has fired on two consecutive rungs):"
  echo "    anchored   = [$(grep -oE "^R33_RC=[0-9]+" /tmp/w289_$tag.out | tail -1)]"
  echo "    unanchored = [$(grep -oh "R33_RC=[0-9]*" /tmp/w289_$tag.out | tr "\n" " ")]"
}

# ★ SYSMEM FIRST: it is the arm under test, and running it first means the vidmem arm cannot
#   be the thing that left the GPU in whatever state the sysmem arm then meets.
run_arm SYSMEM --ce-client-fault
run_arm VIDMEM --ce-client-fault --notifier-vidmem
echo ""
echo "=== W289 NATIVE EXIT rc=0 at $(date -Is) ==="
