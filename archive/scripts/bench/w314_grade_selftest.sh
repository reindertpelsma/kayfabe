#!/usr/bin/env bash
# ★★ KNOWN-POSITIVES FOR `w314_grade.sh` — offline, no GPU, no boot.
#
# A criterion nobody has watched fail is a wish. This drives the grader over crafted fixtures,
# one per pre-registered outcome, and asserts the exit code. ⊘ Note especially fixture 2: an
# ABSENT `PIN-RELEASE` line must grade **2 (UNMEASURED)** and never 0 — that is the single
# distinction w310's criterion C is built around.
set -uo pipefail
SELF=$(cd "$(dirname "$0")" && pwd)
W=$(mktemp -d)
trap 'rm -rf "$W"' EXIT
RC=0

mk() { # mk <tag> <pinrel-line> <reap-line> <cup3val> <guest-dmesg-extra> <hostdmesg-extra>
  local t=$1
  { echo "VAS-PUBLISH token=abc published=3 refused=0 in 5 ms"
    echo "★DRAINED(this doorbell's VAS) asked=10 pinned=7"
    echo "HOST-PUBLISHED host_rows=18295 of 18309"
    [ -n "$2" ] && echo "$2"
    [ -n "$3" ] && echo "$3"
    echo "kayfabe: REAP-TIMING max_reap_us=1234 reaped=1 ⇒ longest"
  } > "$W/run_${t}_qemu.log"
  printf '%s' "$6" > "$W/run_${t}_hostdmesg.log"
  { echo "NVRM: loaded"; [ -n "$5" ] && echo "$5"; } > "$W/run_${t}_dmesg.log"
  { echo "CUP3_VAL=$4"; echo "CUP3_RC=0"; } > "$W/run_${t}_probe.log"
  : > "$W/run_${t}_serial.log"
}

check() { # check <tag> <expected-rc> <what>
  local out rc
  out=$(W314_BENCH="$W" bash "$SELF/w314_grade.sh" "$1" 2>&1); rc=$?
  if [ "$rc" -eq "$2" ]; then
    echo "  ✔ $3 → exit $rc (expected $2)"
  else
    echo "  ✘ $3 → exit $rc, EXPECTED $2"; echo "$out" | tail -20 | sed 's/^/      /'; RC=1
  fi
}

PR='kayfabe: PIN-RELEASE released=12 refused_no_host_vas=0 rows_deduped=3 ⇒ x'
PR0='kayfabe: PIN-RELEASE released=0 refused_no_host_vas=0 rows_deduped=1 ⇒ x'
PRR='kayfabe: PIN-RELEASE released=12 refused_no_host_vas=5 rows_deduped=3 ⇒ x'
RE='kayfabe: REAP reaped=1 still_retired=0 ⇒ x'

echo "=== w314_grade.sh KNOWN-POSITIVES"
mk allgreen "$PR" "$RE" 43 "" "";                       check allgreen 0 "(A–G) all pass"
mk noline   ""    ""   43 "" "";                        check noline   2 "(C) PIN-RELEASE ABSENT ⇒ UNMEASURED, ⊘ not 0 and not a pass"
mk zero     "$PR0" "$RE" 43 "" "";                      check zero     1 "(C) released=0 with the line present ⇒ FAIL"
mk halfwire "$PR" ""   43 "" "";                        check halfwire 1 "(G) PIN-RELEASE without REAP ⇒ half-wired"
mk badval   "$PR" "$RE" 14 "" "";                       check badval   1 "(A) CUP3_VAL=14 ⇒ a copy where a compute belonged"
mk stall    "$PR" "$RE" 43 "BUG: soft lockup - CPU#0 stuck for 23s!" "";  check stall 1 "(D) soft lockup ⇒ the BQL-stall signature"
mk rcu      "$PR" "$RE" 43 "rcu_preempt self-detected stall on CPU" "";   check rcu   1 "(D) RCU stall ⇒ same"
mk xid      "$PR" "$RE" 43 "" 'NVRM: Xid (PCI:0000:01:00): 31, pid=1 CE0';check xid   1 "(F)+(B) a new Xid class"
mk refnv    "$PRR" "$RE" 43 "" "";                      check refnv    1 "(E) refused_no_host_vas=5 ⇒ FAIL"

echo "=== SELFTEST RC=$RC (0 = the grader detects every pre-registered outcome)"
exit "$RC"
