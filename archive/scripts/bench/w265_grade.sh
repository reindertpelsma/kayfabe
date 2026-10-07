#!/usr/bin/env bash
# w265 — extract every pre-registered row from the two arms' logs, MECHANICALLY.
#
# ⊘ It GRADES NOTHING and decides nothing: it prints one row per observable per arm, so the
# scorecard in `traces/boots/w265/RESULT.md` is transcribed from a command's output rather
# than from a reading. A number typed by hand can drift toward the prediction beside it.
#
#   usage: scripts/bench/w265_grade.sh [dir]      (default /workspace/bench)
set -uo pipefail
D=${1:-/workspace/bench}
ARMS="off on"

# ⊘ Absence is `NO-FILE`, never 0. A missing log and a log with none of a thing are different
# facts and only one of them is about the guest — the zero-byte-artefact trap, one instrument over.
# ⊘ `grep -c` PRINTS 0 and EXITS 1 when nothing matches, so a `|| printf '0'` fallback appends a
# SECOND zero and every zero row renders as "0\n0". Measured in w265's own output. The exit status
# is the wrong thing to branch on here: the count is already on stdout.
count() { [ -f "$1" ] || { printf 'NO-FILE'; return; }; printf '%s' "$(grep -c -- "$2" "$1" 2>/dev/null)"; }
countre() { [ -f "$1" ] || { printf 'NO-FILE'; return; }; grep -oE -- "$2" "$1" 2>/dev/null | wc -l; }
first() { [ -f "$1" ] || { printf 'NO-FILE'; return; }; grep -m1 -oE -- "$2" "$1" 2>/dev/null || printf '(none)'; }
last()  { [ -f "$1" ] || { printf 'NO-FILE'; return; }; grep -oE -- "$2" "$1" 2>/dev/null | tail -1 || printf '(none)'; }
# ★ DISTINCT, not occurrences — "8 faults" and "8 DIFFERENT faults" are the exact distinction
# w265 turned on, and `countre` cannot express it.
uniqre() { [ -f "$1" ] || { printf 'NO-FILE'; return; }; printf '%s' "$(grep -oE -- "$2" "$1" 2>/dev/null | sort -u | wc -l)"; }

printf '%-36s' 'observable'
for a in $ARMS; do printf '%-26s' "$a"; done; echo
printf '%s\n' '----------------------------------------------------------------------------------------'

row() { # <label> <fn> <suffix> <pattern>
  printf '%-36s' "$1"
  for a in $ARMS; do printf '%-26s' "$("$2" "$D/run_w265_${a}_$3" "$4")"; done
  echo
}

# ---- ARMING, out of the boot's OWN log. R1 FIRST: everything below is void without it -----
row 'R1  EXEC-WITNESS arm'       first qemu.log 'EXEC-WITNESS (ARMED|DISARMED)'
row 'ARM fb_join'                first qemu.log 'FB-JOIN arm=[a-z]+'
row 'ARM guest_ring'             first qemu.log 'GUEST-RING arm=[a-z]+'
row 'ARM guest_pushbuf'          first qemu.log 'GUEST-PUSHBUF arm=[a-z]+'

# ---- R2..R6 — THE POPULATE SIDE, this rung's own rows -------------------------------------
row 'R2  EXEC-WITNESS by-executor' first qemu.log 'by-executor=[0-9]+'
row 'R2b EXEC-WITNESS resident'    first qemu.log 'resident=[0-9]+'
row 'R3  EXEC-WITNESS refused-cap' first qemu.log 'refused-at-cap=[0-9]+'
row 'R4  VAS-BIND-CENSUS wit= max' last  qemu.log 'wit=[0-9]+'
row 'R4b VAS-BIND wit_sample nonempty' countre qemu.log 'wit_sample=\[0x'
row 'R5  PT-DECODE unwitnessed max' last qemu.log 'unwitnessed=[0-9]+'
row 'R6  PT-DECODE bound max'       last qemu.log ' bound=[0-9]+'
row 'R6b PT-DECODE refusals'        countre qemu.log 'refusals=[1-9][0-9]*'
row 'R6c PT-DECODE faults'          countre qemu.log ' faults=[1-9][0-9]*'
row 'R6d PT-DECODE reach_faults'    countre qemu.log 'reach_faults=[1-9][0-9]*'
row 'R6e PT-DECODE published'       last qemu.log 'published=[0-9]+/[0-9]+'

# ---- R7..R11 — THE CONSUMER --------------------------------------------------------------
row 'R7  PB-PIN table MISS'      countre qemu.log '[1-9][0-9]* MISS'
row 'R7b PB-PIN lines'           count   qemu.log 'PB-PIN token='
row 'R8  PB-PIN resolved-in-GRAM' countre qemu.log '[1-9][0-9]* page\(s\) asked, [1-9][0-9]* resolved'
row 'R8b NOT ONE PAGE RESOLVED'  count   qemu.log 'NOT ONE PAGE RESOLVED IN GUEST RAM'
row 'R9  PB-PIN NOT-IN-GUEST-RAM' countre qemu.log '[1-9][0-9]* NOT-IN-GUEST-RAM'
row 'R10 PB-PIN runs PINNED'     countre qemu.log '→ (PINNED|ALREADY PINNED)'
row 'R10b PB-PIN CAPPED'         count   qemu.log 'CAPPED'
row 'R10c PB-PIN SystemDataPlane' count  qemu.log 'REFUSED SystemDataPlane'
row 'R11 placed_as_asked=true'   countre qemu.log 'placed_as_asked=true'
row 'R11b placed_as_asked=false' countre qemu.log 'placed_as_asked=false'

# ---- R12..R18 — THE HARDWARE ANSWER AND THE GUARDS ---------------------------------------
# ★★★★★ R12 — THE COUNT IS BLIND, AND w265 PROVED IT. `grep -c Xid` read **8 on both arms**
# while the faults changed engine (`CE3_PBDMA0`→`CE3`), client (`HUBCLIENT_ESC`→`HUBCLIENT_CE1`),
# address (8 pushbuffer VAs → 1 semaphore page) and DIRECTION (`VIRT_READ`→`VIRT_WRITE`). Five
# facts, all invisible to a magnitude. ⇒ A COUNT CANNOT SEE A SUBSTITUTION: when a fix is
# expected to MOVE a wall rather than remove it, the identity is the instrument and the count is
# not. The count stays — a fault appearing where there were none is still worth a row — but it is
# NEVER read alone, and the four identity rows below are part of the scorecard, not of the dump.
row 'R12 host Xid COUNT (⊘ blind)' count  hostdmesg.log 'Xid'
row 'R12a Xid ENGINE'            first   hostdmesg.log 'ENGINE [A-Z0-9_]+'
row 'R12b Xid CLIENT'            first   hostdmesg.log 'HUBCLIENT_[A-Z0-9]+'
row 'R12c Xid DISTINCT ADDRS'    uniqre  hostdmesg.log 'faulted @ 0x[0-9a-f_]+'
row 'R12d Xid ACCESS TYPE'       first   hostdmesg.log 'ACCESS_TYPE_[A-Z_]+'
row 'R14 CE-SUBMIT'              count   qemu.log 'CE-SUBMIT'
row 'R14b RETIRED'               count   qemu.log 'RETIRED'
row 'R15 RmInitAdapter failed'   count   dmesg.log 'RmInitAdapter failed'
row 'R16 guest NVRM'             count   dmesg.log 'NVRM'
row 'R16b GR-BIRTH'              count   qemu.log 'GR-BIRTH'
row 'R17 ENGINE-OBJECT census'   first   qemu.log 'seen=[0-9]+ forwarded=[0-9]+ refused=[0-9]+'
row 'R18 BAR1 GP_PUT'            countre qemu.log 'GP_PUT'
row '..  RING-PROJ'              count   qemu.log 'RING-PROJ token='
row '..  adopt=GUEST-RING'       countre qemu.log 'adopt=GUEST-RING'
row '..  userd=GUEST-USERD'      countre qemu.log 'userd=GUEST-USERD'
row '..  ring-pin NOT IN GUEST'  count   qemu.log 'NOT IN GUEST RAM'
row '..  fbuserd GET nonzero'    countre qemu.log 'fbuserd@0x[0-9a-f]+ GET=[1-9]'
row '..  ENOSPC/LLVM (SAME log)' countre qemu.log 'No space left on device|LLVM ERROR'

printf '%-36s' 'R13 CUP2_RC'
for a in $ARMS; do printf '%-26s' "$(first "$D/run_w265_${a}_probe.log" 'CUP2_RC=[0-9]+')"; done
echo

echo
echo '=== R4 — EVERY VAS-BIND-CENSUS wit=/published=, per arm (the DIRECT readout) ==='
for a in $ARMS; do
  printf '%s: ' "$a"
  grep -oE 'pdb=0x[0-9a-f]+ .*wit=[0-9]+ published=[0-9]+ wit_sample=\[[^]]*\]' \
    "$D/run_w265_${a}_qemu.log" 2>/dev/null | sed 's/va=0x[0-9a-f]* //' | sort -u | head -6
  echo
done

echo
echo '=== R5/R6 — EVERY PT-DECODE TALLY, per arm ==='
for a in $ARMS; do
  printf '%s:\n' "$a"
  grep -oE 'PT-DECODE drained=.*' "$D/run_w265_${a}_qemu.log" 2>/dev/null | cut -c1-230 | sed 's/^/    /'
done

echo
echo '=== R7..R10 — THE PB-PIN LINES IN FULL, per arm ==='
for a in $ARMS; do
  printf '%s:\n' "$a"
  grep -A3 'PB-PIN token=' "$D/run_w265_${a}_qemu.log" 2>/dev/null | head -50 | sed 's/^/    /'
done

echo
echo '=== R12 — THE HOST Xid ADDRESSES, per arm ==='
for a in $ARMS; do
  printf '%s: ' "$a"
  grep -oE 'faulted @ 0x[0-9a-f_]+' "$D/run_w265_${a}_hostdmesg.log" 2>/dev/null | sort -u | tr '\n' ' '
  echo
done

echo
echo '=== THE gp[] ENTRIES AND THEIR APERTURES, per arm ==='
for a in $ARMS; do
  printf '%s:\n' "$a"
  grep -oE 'gp\[[0-9]+\]@0x[0-9a-f]+=0x[0-9a-f]+\+0x[0-9a-f]+ pb=[^ ]+' \
    "$D/run_w265_${a}_qemu.log" 2>/dev/null | sort -u | sed 's/^/    /'
done
