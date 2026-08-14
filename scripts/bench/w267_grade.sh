#!/usr/bin/env bash
# w267 — READ THE PAGE. Extract every pre-registered row from the two arms' logs, MECHANICALLY.
#
# ⊘ It GRADES NOTHING and decides nothing: it prints one row per observable per arm, so the
# scorecard in `traces/boots/w267/RESULT.md` is transcribed from a command's output rather
# than from a reading. A number typed by hand can drift toward the prediction beside it.
#
#   usage: scripts/bench/w267_grade.sh [dir]      (default /workspace/bench)
set -uo pipefail
D=${1:-/workspace/bench}
ARMS="off on"

# ⊘ Absence is `NO-FILE`, never 0. A missing log and a log with none of a thing are different
# facts and only one of them is about the guest — the zero-byte-artefact trap, one instrument over.
# ⊘ `grep -c` PRINTS 0 and EXITS 1 when nothing matches, so a `|| printf '0'` fallback appends a
# SECOND zero and every zero row renders as "0\n0". Measured in w266's own output. The exit status
# is the wrong thing to branch on here: the count is already on stdout.
count() { [ -f "$1" ] || { printf 'NO-FILE'; return; }; printf '%s' "$(grep -c -- "$2" "$1" 2>/dev/null)"; }
countre() { [ -f "$1" ] || { printf 'NO-FILE'; return; }; grep -oE -- "$2" "$1" 2>/dev/null | wc -l; }
first() { [ -f "$1" ] || { printf 'NO-FILE'; return; }; grep -m1 -oE -- "$2" "$1" 2>/dev/null || printf '(none)'; }
last()  { [ -f "$1" ] || { printf 'NO-FILE'; return; }; grep -oE -- "$2" "$1" 2>/dev/null | tail -1 || printf '(none)'; }
# ★ DISTINCT, not occurrences — "8 faults" and "8 DIFFERENT faults" are the exact distinction
# w266 turned on, and `countre` cannot express it.
uniqre() { [ -f "$1" ] || { printf 'NO-FILE'; return; }; printf '%s' "$(grep -oE -- "$2" "$1" 2>/dev/null | sort -u | wc -l)"; }

printf '%-36s' 'observable'
for a in $ARMS; do printf '%-26s' "$a"; done; echo
printf '%s\n' '----------------------------------------------------------------------------------------'

row() { # <label> <fn> <suffix> <pattern>
  printf '%-36s' "$1"
  for a in $ARMS; do printf '%-26s' "$("$2" "$D/run_w267_${a}_$3" "$4")"; done
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
# ★★ R6f/R6g — THE CROSS-REVISION GUARD, and it must be the FIRST pass, not the last.
# ⊘ `R5`/`R6` above use `last`, and the final `PT-DECODE` line of every boot reads zero because
# only the first pass decodes — `w266` §2.4 states this as a known artifact and it was carried
# rather than fixed, so both readings are kept and labelled. THESE are the numbers w266 pinned
# (`bound=19615 unwitnessed=19874`); if they move, this binary is doing different work from
# w266's and the arm-to-arm comparison across rungs is VOID.
row 'R6f PT-DECODE bound (1st)'    first qemu.log '→ bound=[0-9]+'
row 'R6g PT-DECODE unwitn (1st)'   first qemu.log 'unwitnessed=[0-9]+'

# ---- R7..R11 — THE CONSUMER --------------------------------------------------------------
row 'R7  PB-PIN table MISS'      countre qemu.log '[1-9][0-9]* MISS'
row 'R7b PB-PIN lines'           count   qemu.log 'PB-PIN token='
# ⊘⊘ THREE ROWS WERE CONTAMINATED BY LEG 5 AND WERE FIXED AT w266 — measured in w266's own
# output. `R8`/`R10`/`R11` were written when `pin_pushbuffer_guest_ram` was the ONLY producer of
# `TABLE:` / `PINNED` / `placed_as_asked=`, so their regexes were scoped by that fact and not by
# their text. `pin_completion_guest_ram` prints the same shapes, and the `on` arm read 16/16/20
# against the control's 8/8/12 — reading as "leg 4 doubled" when leg 4 did not move at all
# (`R7b PB-PIN token=` is 16 on BOTH arms, which is what exonerates it).
# ★★★ ADDING A PRODUCER SILENTLY RE-SCOPES EVERY CONSUMER THAT WAS IMPLICITLY SCOPED BY BEING
# THE ONLY ONE. Every row below is now anchored to `pb run` / `PB-PIN`, so a THIRD pin source
# cannot repeat this.
# ⊘ `[^-]TABLE:` and not `TABLE:` — `SEMA-TABLE:` CONTAINS `TABLE:`, so the obvious anchor
# would have left this row contaminated while looking fixed. The character before the label is
# a space for leg 4 and a `-` for leg 5.
row 'R8  PB-PIN resolved-in-GRAM' countre qemu.log '[^-]TABLE: [1-9][0-9]* page\(s\) asked, [1-9][0-9]* resolved'
row 'R8b NOT ONE PAGE RESOLVED'  countre qemu.log '⊘ NOT ONE PAGE RESOLVED IN GUEST RAM'
row 'R9  PB-PIN NOT-IN-GUEST-RAM' countre qemu.log 'pb run [0-9]+/[0-9]+.*NOT-IN-GUEST-RAM'
row 'R10 PB-PIN runs PINNED'     countre qemu.log 'pb run [0-9]+/[0-9]+.*(→ PINNED|ALREADY PINNED)'
row 'R10b PB-PIN CAPPED'         count   qemu.log 'CAPPED'
row 'R10c PB-PIN SystemDataPlane' count  qemu.log 'REFUSED SystemDataPlane'
row 'R11 placed_as_asked=true'   countre qemu.log 'pb run [0-9]+/[0-9]+.*placed_as_asked=true'
row 'R11b placed_as_asked=false' countre qemu.log 'placed_as_asked=false'

# ---- S1..S13 — LEG 5's OWN ROWS, the completion-semaphore pin -----------------------------
# ⊘ S2's CONTROL EXPECTATION IS AN ABSENCE, and it is graded as a NUMBER for that reason: the
# disarmed arm must print ZERO `SEMA-PIN` lines, which is also what an older binary prints. The
# content check in `w267_run.sh` is what separates those two, not this row.
row 'S1  ARM guest_sema'         first   qemu.log 'GUEST-SEMA arm=[a-z]+'
row 'S2  SEMA-PIN lines'         count   qemu.log 'SEMA-PIN token='
row 'S3  SEMA-PIN declared'      first   qemu.log 'SOURCE [0-9]+ declared completion'
row 'S4  SEMA-PIN pages dedup'   first   qemu.log '[0-9]+ page\(s\) after de-duplication'
row 'S5  SEMA resolved'          first   qemu.log 'SEMA-TABLE: [0-9]+ page\(s\) asked, [0-9]+ resolved in guest RAM'
row 'S6  SEMA MISS nonzero'      countre qemu.log 'SEMA-TABLE:.*[1-9][0-9]* MISS'
row 'S7  SEMA NOT-IN-GRAM'      countre qemu.log 'SEMA-TABLE:.*[1-9][0-9]* NOT-IN-GUEST-RAM'
row 'S8  sema run PINNED fresh'  countre qemu.log 'sema run [0-9]+/[0-9]+.*→ PINNED'
row 'S9  sema run ALREADY'       countre qemu.log 'sema run [0-9]+/[0-9]+.*ALREADY PINNED'
row 'S9b sema run REFUSED'       countre qemu.log 'sema run [0-9]+/[0-9]+.*REFUSED'
row 'S10 SEMA placed_as_asked'   countre qemu.log 'sema run [0-9]+/[0-9]+.*placed_as_asked=true'
row 'S11 SEMA neg-control fired' countre qemu.log 'SEMAPHORE RUN\(S\) PLACED.*NEGATIVE CONTROL: gpa=0x[0-9a-f]+ \(one'
row 'S11b NO PAGE TO PIN'        count   qemu.log 'NO PAGE TO PIN'
row 'S11c SEMA NOT ONE PAGE'     count   qemu.log 'SEMA: NOT ONE PAGE RESOLVED'
# ★★★★★ S13 — THE ROW THAT SEPARATES "the page is writable" FROM "the guest's wait was
# satisfied". Only OBSERVED can precede CUP2_RC moving; a green pin beside NOT-OBSERVED is the
# separation itself, and it is pre-registered as the likely outcome.
row 'S13 COMPLETION-WATCH OBSERVED' countre qemu.log 'COMPLETION-WATCH.*→ OBSERVED'
row 'S13b COMPLETION-WATCH NOT-OBS' countre qemu.log 'COMPLETION-WATCH.*NOT-OBSERVED'
row 'S13c COMPLETION-WATCH last_seen' last qemu.log 'last_seen=0x[0-9a-f]+'
row 'S13d COMPLETION-DECLARE'    count   qemu.log 'COMPLETION-DECLARE'

# ---- P1..P8 — ★★★★★ THIS RUNG'S OWN ROWS: THE 4 KiB PAGE ---------------------------------
# ⊘ P1/P2 FIRST and they are about the INSTRUMENT, not the guest. Every row below is void
# without them: an absent SEMA-PAGE row means the reader never ran, which is NOT an empty page.
# The two answers this rung exists to separate are exactly "the page is empty" and "nothing
# looked", and they produce the same silence.
row 'P1  SEMA-PAGE dumps'        count   qemu.log 'SEMA-PAGE seq='
row 'P2  SEMA-PAGE-READER tally' first   qemu.log 'looks=[0-9]+ dumps_printed=[0-9]+ dumps_suppressed=[0-9]+ pages=[0-9]+'
row 'P2b SEMA-PAGE READ-REFUSED' count   qemu.log 'SEMA-PAGE seq=.*READ-REFUSED'
# ★★★ P3 — THE ROW THAT DECIDES (a) vs (b). `nonzero=0/1024` on every dump is world (b): the
# page became writable and NOTHING observable was written to it. Any other value is world (a)
# or (c), and P5 says at which offset.
row 'P3  nonzero= (first dump)'  first   qemu.log 'nonzero=[0-9]+/[0-9]+'
row 'P3b nonzero= (last dump)'   last    qemu.log 'nonzero=[0-9]+/[0-9]+'
row 'P3c DISTINCT nonzero= seen' uniqre  qemu.log 'nonzero=[0-9]+/[0-9]+'
row 'P4  ALL-ZERO dumps'         count   qemu.log 'SEMA-PAGE-ZERO'
row 'P5  non-zero slot lines'    count   qemu.log 'SEMA-PAGE-NZ'
row 'P5b listing bounded'        count   qemu.log 'LISTING-BOUND'
# ⊘ P6 — DISTINCT content signatures. >1 means the page CHANGED during the run, which is the
# strongest form of (a): a SEQUENCE of writes, with t=+Nms giving the rate.
row 'P6  DISTINCT page sigs'     uniqre  qemu.log 'sig=0x[0-9a-f]{16}'
row 'P6b dumps with changed=1'   countre qemu.log 'changed=1'
row 'P7  SEMA-PAGE-SLOT rows'    count   qemu.log 'SEMA-PAGE-SLOT'
# ★★ P8 — THE CE's OWN SEMAPHORE OPERAND, un-truncated. `n3=[0x2,0xB,0xPAYLOAD]` is the
# address hardware faulted on, and it was on disk and unread at w266.
row 'P8  CE SET_SEMAPHORE run'   first   qemu.log 'm0x240/[A-Za-z]+/n3=\[[^]]*\]'
row 'P8b DISTINCT m0x240 operands' uniqre qemu.log 'm0x240/[A-Za-z]+/n3=\[[^]]*\]'
row 'P8c CE LAUNCH_DMA arg'      first   qemu.log 'm0x300/[A-Za-z]+/n1=\[[^]]*\]'
row 'P8d pbm runs SHORT'         count   qemu.log '/SHORT-'

# ---- R12..R18 — THE HARDWARE ANSWER AND THE GUARDS ---------------------------------------
# ★★★★★ R12 — THE COUNT IS BLIND, AND w266 PROVED IT. `grep -c Xid` read **8 on both arms**
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
for a in $ARMS; do printf '%-26s' "$(first "$D/run_w267_${a}_probe.log" '^CUP2_RC=[0-9]+')"; done
echo

echo
echo '=== R4 — EVERY VAS-BIND-CENSUS wit=/published=, per arm (the DIRECT readout) ==='
for a in $ARMS; do
  printf '%s: ' "$a"
  grep -oE 'pdb=0x[0-9a-f]+ .*wit=[0-9]+ published=[0-9]+ wit_sample=\[[^]]*\]' \
    "$D/run_w267_${a}_qemu.log" 2>/dev/null | sed 's/va=0x[0-9a-f]* //' | sort -u | head -6
  echo
done

echo
echo '=== R5/R6 — EVERY PT-DECODE TALLY, per arm ==='
for a in $ARMS; do
  printf '%s:\n' "$a"
  grep -oE 'PT-DECODE drained=.*' "$D/run_w267_${a}_qemu.log" 2>/dev/null | cut -c1-230 | sed 's/^/    /'
done

echo
echo '=== R7..R10 — THE PB-PIN LINES IN FULL, per arm ==='
for a in $ARMS; do
  printf '%s:\n' "$a"
  grep -A3 'PB-PIN token=' "$D/run_w267_${a}_qemu.log" 2>/dev/null | head -50 | sed 's/^/    /'
done

echo
echo '=== R12 — THE HOST Xid ADDRESSES, per arm ==='
for a in $ARMS; do
  printf '%s: ' "$a"
  grep -oE 'faulted @ 0x[0-9a-f_]+' "$D/run_w267_${a}_hostdmesg.log" 2>/dev/null | sort -u | tr '\n' ' '
  echo
done

echo
echo '=== THE gp[] ENTRIES AND THEIR APERTURES, per arm ==='
for a in $ARMS; do
  printf '%s:\n' "$a"
  grep -oE 'gp\[[0-9]+\]@0x[0-9a-f]+=0x[0-9a-f]+\+0x[0-9a-f]+ pb=[^ ]+' \
    "$D/run_w267_${a}_qemu.log" 2>/dev/null | sort -u | sed 's/^/    /'
done

echo
echo '=== S2..S11 — THE SEMA-PIN LINES IN FULL, per arm ==='
for a in $ARMS; do
  printf '%s:\n' "$a"
  grep -A3 'SEMA-PIN token=' "$D/run_w267_${a}_qemu.log" 2>/dev/null | head -60 | sed 's/^/    /'
done

echo
echo '=== S13 — EVERY COMPLETION-WATCH VERDICT, per arm (⊘ the row CUP2_RC needs) ==='
for a in $ARMS; do
  printf '%s:\n' "$a"
  grep -oE 'COMPLETION-WATCH proc=[0-9]+ chan=[0-9]+ va=0x[0-9a-f]+ payload=0x[0-9a-f]+ → [A-Z-]+ samples=[0-9]+ last_seen=0x[0-9a-f]+' \
    "$D/run_w267_${a}_qemu.log" 2>/dev/null | sed 's/^/    /'
done

echo
echo '=== ★★★★★ P — THE PAGE ITSELF. The FIRST dump, a MIDDLE one and the LAST, per arm ==='
echo '    ⊘ Three instants, not one: a single dump cannot distinguish "never written" from'
echo '      "written and overwritten". Each carries t=+Nms, and the COMPLETION-WATCH verdicts'
echo '      above are in the same log in the same order.'
for a in $ARMS; do
  printf '%s:\n' "$a"
  f="$D/run_w267_${a}_qemu.log"
  [ -f "$f" ] || { echo '    NO-FILE'; continue; }
  # ⊘ Blocks, not lines: a dump is a header plus its slot rows and they are only readable
  # together. `-A 24` covers 8 SEMA-PAGE-SLOT rows + the zero/NZ rows with room to spare.
  echo '    --- FIRST dump:'
  grep -A24 'SEMA-PAGE seq=' "$f" 2>/dev/null | head -26 | sed 's/^/      /'
  echo '    --- LAST dump (why=final):'
  awk '/SEMA-PAGE seq=.*why=final/{p=1} p' "$f" 2>/dev/null | head -26 | sed 's/^/      /'
  echo '    --- EVERY DISTINCT non-zero slot line seen in the whole boot:'
  grep -o 'SEMA-PAGE-NZ .*' "$f" 2>/dev/null | sed 's/gpa=0x[0-9a-f]* //' | sort -u | head -40 | sed 's/^/      /'
  echo '    --- EVERY DISTINCT report16 (the 8 declared slots, zero or not):'
  grep -o 'SEMA-PAGE-SLOT .*' "$f" 2>/dev/null | sed 's/^.*va=/va=/' | sort -u | head -20 | sed 's/^/      /'
  echo
done

echo
echo '=== ★★★ P8 — THE CE CHANNELS'"'"' OWN PUSHBUFFER, EVERY ARGUMENT, per arm ==='
echo '    ⊘ At w266 the m0x240 run printed as "=0x2" — the SET_SEMAPHORE_A half alone. The'
echo '      low 32 bits (the whole offset within the faulting page) and the payload were'
echo '      dropped at the print. Address = ((A & 0x1ffff) << 32) | B  [clc7b5.h:47-52].'
for a in $ARMS; do
  printf '%s:\n' "$a"
  grep -o 'pbm\[[^]]*\]:.*' "$D/run_w267_${a}_qemu.log" 2>/dev/null \
    | sed 's/ (projection.*//' | sort -u | head -12 | sed 's/^/      /'
done
