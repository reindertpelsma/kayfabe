#!/usr/bin/env bash
# ★★★★★ w329 — N boots of ONE arm, printing the RELEASE's own trajectory.
#
# Descends from `w326_arm.sh` and keeps every one of its reasons verbatim: one boot is not a
# grade, every metric is anchored, an absent metric prints `⊘UNMEASURED` and NEVER `0`, and a
# TERMINATOR line is written last so `143` (the job itself was killed) and `124` (the LAUNCHER
# expired while the job ran on) are distinguishable from a clean exit.
#
# ★★★ WHAT IS NEW, and it is the PRE-REGISTERED FALSIFIER ITSELF:
#
#   JOINREL   — `revoked= released= stranded= drained= joined_ranges= still_desired=` off the
#               `PT-DECODE`/`PT-SWEEP` lines, plus the ARM the boot actually ran.
#   JOINTRAJ  — ★★★★★ the `joined_ranges=` TRAJECTORY: n, first, max, last, and **how many
#               times it FELL**. `w327` measured it climbing `0 → 83` over nine allocate/free
#               cycles and **never once falling**; a green `28,31` with `falls=0` means the
#               failure was MASKED, not fixed. ⊘ This is graded on `falls`, never on the
#               final value: a run that ends low because it never joined anything at all is
#               not the same fact and a single number cannot tell them apart.
#
#   usage: w329_arm.sh <TREE> <PREFIX> <N> [env assignments...]
#          W329_WORKLOAD="w308_cup8.sh cup8"   selects a different workload script
set -uo pipefail
TREE=$1; PREFIX=$2; N=${3:-3}; shift 3 || true
LOG=/workspace/w329_arm_${PREFIX}.log
export PATH=/root/.cargo/bin:$PATH
export KAYFABE_REPO=$TREE
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-$(basename "$TREE")}
for kv in "$@"; do export "${kv?}"; done

{
echo "=== W329 ARM START tree=$TREE prefix=$PREFIX n=$N $(date -Is)"
echo "=== HEAD=[$(cd "$TREE" && git rev-parse --short HEAD 2>/dev/null || echo '⊘ NO REPO — UNATTRIBUTABLE')]"
echo "=== DIRT=[$(cd "$TREE" && git status --porcelain --untracked-files=no 2>/dev/null | head -3)]"
echo "=== ARM ENV (intent — the boot's own lines are the record of EXECUTION):"
for kv in "$@"; do echo "===   $kv"; done
echo "===   KAYFABE_JOIN_RELEASE=[${KAYFABE_JOIN_RELEASE:-⊘unset ⇒ ON ⇒ the w329 fix}]"
echo "===   KAYFABE_BENCH_BW=[${KAYFABE_BENCH_BW:-⊘unset}]"
for i in $(seq 1 "$N"); do
  export KAYFABE_TAG=${PREFIX}$i
  pkill -x qemu-system-x86 2>/dev/null; sleep 4
  echo "--- boot $i/$N tag=$KAYFABE_TAG $(date -Is)"
  # shellcheck disable=SC2086
  bash "$TREE/scripts/bench/"${W329_WORKLOAD:-w297_cup3.sh} >/dev/null 2>&1
  P=/workspace/bench/run_${KAYFABE_TAG}_probe.log
  Q=/workspace/bench/run_${KAYFABE_TAG}_qemu.log
  D=/workspace/bench/run_${KAYFABE_TAG}_hostdmesg.log
  G=/workspace/bench/run_${KAYFABE_TAG}_dmesg.log
  V=$(grep -aoE '^CUP3_VAL=[A-Za-z0-9_]+' "$P" 2>/dev/null | tail -1)
  R=$(grep -aoE '^CUP3_RC=[0-9]+' "$P" 2>/dev/null | tail -1)
  C8=$(grep -aoE '^CUP8_(BAD|MAXERR)=[0-9]+' "$P" 2>/dev/null | tr '\n' ' ')
  A1=$(grep -a '★     R33 arm 1 COPY' "$P" 2>/dev/null | tail -1)
  # ---- ★★★★★ THE BW LADDER (the `28,31` repro's own answer, from the WORKLOAD's words)
  OK=$(grep -ah 'BWROW ' "$P" 2>/dev/null | grep -a 'read_GBps=' | sed -n 's/.*BWROW mib=\([0-9]*\) .*/\1/p' | sort -n | tail -1)
  FA=$(grep -ah 'BWROW ' "$P" 2>/dev/null | grep -a 'UNMEASURED'  | sed -n 's/.*BWROW mib=\([0-9]*\) .*/\1/p' | sort -n | head -1)
  FF=$(grep -ah 'BW_FILL_FAIL\|BW_ALLOC_FAIL' "$P" 2>/dev/null | head -1)
  # ---- ★★★★★ THE RELEASE'S OWN LINES
  JA=$(grep -ao 'JOIN-RELEASE arm=[a-z]*' "$Q" 2>/dev/null | sort -u | tr '\n' ' ')
  JS=$(grep -ao 'revoked=[0-9]* released=[0-9]* stranded=[0-9]* drained=[0-9]*' "$Q" 2>/dev/null \
        | awk '{for(i=1;i<=NF;i++){split($i,a,"=");s[a[1]]+=a[2]}} END{printf "revoked=%d released=%d stranded=%d drained=%d",s["revoked"],s["released"],s["stranded"],s["drained"]}')
  SD=$(grep -ao 'still_desired=[0-9]*' "$Q" 2>/dev/null | sed 's/.*=//' | awk '{s+=$1} END{print s+0}')
  DIS=$(grep -ac 'TABLE/STORE DISAGREE' "$Q" 2>/dev/null)
  # ---- ★★★★★ THE TRAJECTORY, and it is graded on `falls`
  TRAJ=$(grep -ao 'joined_ranges=[0-9]*' "$Q" 2>/dev/null | sed 's/.*=//' \
        | awk 'NR==1{f=$1;mx=$1;mn=$1} {if($1>mx)mx=$1; if($1<mn)mn=$1; if(NR>1&&$1<p)fell++; p=$1; l=$1; n++}
               END{if(n==0){print "⊘UNMEASURED — the JOIN-RELEASE clause never printed"}
                   else printf "n=%d first=%d max=%d min=%d last=%d falls=%d", n,f,mx,mn,l,fell+0}')
  # ---- the pre-existing anchors, unchanged from w326_arm.sh
  ALJ=$(grep -ac 'already joined' "$Q" 2>/dev/null)
  DR=$(grep -ao 'DRAIN\[visited=true asked=[0-9]* pinned=[0-9]* refused=[0-9]* DRAIN_MS=[0-9]* W319KNOB\[[^]]*\] complete=[a-z]*[^]]*' "$Q" 2>/dev/null | head -1)
  HR=$(grep -ao 'host_rows[= ][0-9]*/[0-9]*' "$Q" 2>/dev/null | tail -1)
  X=$(grep -ac Xid "$D" 2>/dev/null)
  XF=$(grep -aoE 'Xid \(PCI:[^)]*\): *[0-9]+|ENGINE [A-Z0-9_]+|HUBCLIENT_[A-Z0-9_]+|faulted @ 0x[0-9a-f_]+|FAULT_[A-Z]+[0-9]*|ACCESS_TYPE_[A-Z_]+' "$D" 2>/dev/null | tr '\n' ' ')
  UN=$(grep -ho 'unserviced fn 76 cmd 0x[0-9a-f]*' "$Q" 2>/dev/null | sort -u | wc -l)
  NV=$(grep -ac NVRM "$G" 2>/dev/null)
  BM=$(grep -aoE 'BENCH_MODE=[A-Z]+' "$P" 2>/dev/null | tail -1)
  NB=$(grep -ah 'BENCH_NOLAUNCH_TOTAL_BAD=' "$P" 2>/dev/null | tail -1)
  echo "    BOOT $i: [${V:-⊘ABSENT-UNMEASURED}] [${R:-⊘NO-TERMINATOR}] [${C8:-⊘no-cup8}] [${BM:-⊘no-BENCH_MODE}]"
  echo "            r33_arm1=[${A1:-⊘ NO arm-1 LINE — the measurement did not happen, ⊘ NOT a fail}]"
  echo "            BW last_ok=[${OK:-⊘NONE}] first_fail=[${FA:-⊘NONE_FAILED}] ${FF:+refusal=[$FF]}"
  echo "            ★★★★★ JOINREL arm=[${JA:-⊘ABSENT — the clause never printed, UNMEASURED}] ${JS:-⊘UNMEASURED} still_desired=[${SD:-⊘}] table_store_disagree=[${DIS:-⊘NOFILE}]"
  echo "            ★★★★★ JOINTRAJ ${TRAJ}"
  echo "            already_joined_refusals=[${ALJ:-⊘NOFILE}] (w327 baseline 16, failing boots 26-32)"
  echo "            ★★★ DRAIN=[${DR:-⊘ NO DRAIN CLAUSE — UNMEASURED, ⊘ not complete}]"
  echo "            ${HR:-⊘host_rows UNMEASURED}  unserviced_distinct=[${UN:-⊘NOFILE}]"
  echo "            Xid_lines=[${X:-⊘NOFILE}] ${XF}"
  echo "            guest NVRM_lines=[${NV:-⊘NOFILE}] ${NB:+${NB}}"
done
echo "=== W329 ARM EXIT status=0 $(date -Is)"
} >"$LOG" 2>&1
echo "=== W329 ARM TERMINATOR $PREFIX rc=0 $(date -Is)" >>"$LOG"
