#!/usr/bin/env bash
# w264 — extract every pre-registered row from the four arms' logs, mechanically.
#
# ⊘ It GRADES NOTHING and decides nothing: it prints one row per observable per arm, so the
# scorecard in `traces/boots/w264/RESULT.md` is transcribed from a command's output rather
# than from a reading. A number typed by hand is a number that can drift toward the
# prediction beside it.
#
#   usage: scripts/bench/w264_grade.sh [dir]      (default /workspace/bench)
set -uo pipefail
D=${1:-/workspace/bench}
ARMS="base join ring pin"

# ⊘ Absence is reported as `NO-FILE`, never as 0. A missing log and a log with none of a
# thing are different facts and only one of them is about the guest — the zero-byte-artefact
# trap, one instrument over.
count() { # <file> <pattern>
  [ -f "$1" ] || { printf 'NO-FILE'; return; }
  grep -c -- "$2" "$1" 2>/dev/null || printf '0'
}
countre() { # <file> <extended regex>  — counts OCCURRENCES, not lines
  [ -f "$1" ] || { printf 'NO-FILE'; return; }
  grep -oE -- "$2" "$1" 2>/dev/null | wc -l
}
first() { # <file> <extended regex>
  [ -f "$1" ] || { printf 'NO-FILE'; return; }
  grep -m1 -oE -- "$2" "$1" 2>/dev/null || printf '(none)'
}

printf '%-34s' 'observable'
for a in $ARMS; do printf '%-14s' "$a"; done; echo
printf '%s\n' '--------------------------------------------------------------------------------'

row() { # <label> <fn> <suffix> <pattern>
  printf '%-34s' "$1"
  for a in $ARMS; do printf '%-14s' "$("$2" "$D/run_w264_${a}_$3" "$4")"; done
  echo
}

# ---- ARMING, read out of the boot's OWN log rather than out of any shell -----------------
row 'ARM fb_join'        first qemu.log  'FB-JOIN arm=[a-z]+'
row 'ARM guest_ring'     first qemu.log  'GUEST-RING arm=[a-z]+'
row 'ARM guest_pushbuf'  first qemu.log  'GUEST-PUSHBUF arm=[a-z]+'

# ---- Q1..Q6 — LEG 4's own rows ----------------------------------------------------------
row 'Q1  PB-PIN lines'           count    qemu.log 'PB-PIN token='
row 'Q2  PB-PIN NOT-IN-GUEST-RAM' countre qemu.log '[1-9][0-9]* NOT-IN-GUEST-RAM'
row 'Q3  PB-PIN runs PINNED'     countre qemu.log '→ (PINNED|ALREADY PINNED)'
row 'Q4  placed_as_asked=false'  countre qemu.log 'placed_as_asked=false'
row 'Q4b placed_as_asked=true'   countre qemu.log 'placed_as_asked=true'
row 'Q5  PB-PIN table MISS'      countre qemu.log '[1-9][0-9]* MISS'
row 'Q5b NOT ONE PAGE RESOLVED'  count    qemu.log 'NOT ONE PAGE RESOLVED IN GUEST RAM'
row 'Q6  CAPPED'                 count    qemu.log 'CAPPED'
row 'Q6b PB-PIN NO EXTENT'       count    qemu.log 'NO EXTENT NAMED'
row 'Q6c PB-PIN SystemDataPlane' count    qemu.log 'REFUSED SystemDataPlane'

# ---- Q7..Q16 — the inherited rows and the guards -----------------------------------------
row 'Q7  host Xid'               count    hostdmesg.log 'Xid'
row 'Q8  fbuserd GET nonzero'    countre  qemu.log 'fbuserd@0x[0-9a-f]+ GET=[1-9]'
row 'Q9  ring-pin NOT IN GUEST'  count    qemu.log 'NOT IN GUEST RAM'
row 'Q10 adopt=GUEST-RING'       countre  qemu.log 'adopt=GUEST-RING'
row 'Q11 userd=GUEST-USERD'      countre  qemu.log 'userd=GUEST-USERD'
row 'Q12 RmInitAdapter failed'   count    dmesg.log 'RmInitAdapter failed'
row 'Q14 CE-SUBMIT'              count    qemu.log 'CE-SUBMIT'
row 'Q14b RETIRED'               count    qemu.log 'RETIRED'
row 'Q15 ENGINE-OBJECT census'   first    qemu.log 'seen=[0-9]+ forwarded=[0-9]+ refused=[0-9]+'
row 'Q16 BAR1 GP_PUT'            countre  qemu.log 'GP_PUT'
row '..  RING-PROJ'              count    qemu.log 'RING-PROJ token='
row '..  ENOSPC/LLVM (SAME log)' countre  qemu.log 'No space left on device|LLVM ERROR'

printf '%-34s' 'Q13 CUP2_RC'
for a in $ARMS; do printf '%-14s' "$(first "$D/run_w264_${a}_probe.log" 'CUP2_RC=[0-9]+')"; done
echo

echo
echo '=== THE PB-PIN LINES IN FULL (pin arm) ==='
sed -n '/PB-PIN token=/,/^kayfabe: [A-Z]/p' "$D/run_w264_pin_qemu.log" 2>/dev/null | head -60

echo
echo '=== THE HOST Xid ADDRESSES, per arm ==='
for a in $ARMS; do
  printf '%s: ' "$a"
  grep -oE 'faulted @ 0x[0-9a-f_]+' "$D/run_w264_${a}_hostdmesg.log" 2>/dev/null | sort -u | tr '\n' ' '
  echo
done

echo
echo '=== THE gp[] ENTRIES AND THEIR APERTURES, per arm ==='
for a in $ARMS; do
  printf '%s:\n' "$a"
  grep -oE 'gp\[[0-9]+\]@0x[0-9a-f]+=0x[0-9a-f]+\+0x[0-9a-f]+ pb=[^ ]+' \
    "$D/run_w264_${a}_qemu.log" 2>/dev/null | sort -u | sed 's/^/    /'
done
