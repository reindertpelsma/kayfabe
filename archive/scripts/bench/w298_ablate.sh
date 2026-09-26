#!/usr/bin/env bash
# ★★★★★ w298 — ABLATE THE ELEVEN RELAXATIONS, ONE AT A TIME, AGAINST THE `43` KNOWN-POSITIVE.
#
#   usage: w298_ablate.sh <label> [VAR=VALUE ...]
#
#   `w298_ablate.sh base`                        — the w297 arming, byte for byte.
#   `w298_ablate.sh ptsweep KAYFABE_PT_SWEEP=off` — one variable moved, ten held.
#
# ## Why this script exists at all
#
#   `w290p_run.sh` set all eleven with UNCONDITIONAL `export`s, so an ablation was not
#   expressible from outside it. w298 made them `${V:-default}`; this file is the caller that
#   uses that, and the ONLY place an arm is chosen. One variable per boot, by construction:
#   the script refuses more than one override unless `W298_MULTI=1` is set, because a two-
#   variable boot produces an unattributable outcome and this campaign has paid for that.
#
# ## ★★★ THE PRE-REGISTERED READING — written before any boot, so no outcome reads as the
#     favourable one after the fact
#
#   (A) `^CUP3_VAL=43` with the variable OFF ⇒ ★★★★★ THE RELAXATION IS INERT. It is a
#       CANDIDATE FOR DELETION FROM MASTER, not a null result. We have been carrying it.
#   (B) `^CUP3_VAL=` != 43 with the variable OFF ⇒ the relaxation is LOAD-BEARING. ⊘ "It went
#       red" is NOT the finding. The finding is WHAT IT WAS HOLDING UP, BY IDENTITY: the
#       ladder stage, the Xid (engine / client / access / descent level), the fault VA and
#       whether our own table describes it, and the refusal NAME if one fired.
#   (C) `^CUP3_VAL=` ABSENT ⇒ ⊘ UNMEASURED. NOT 0, NOT a failure value, and NOT evidence the
#       variable is load-bearing. Say so and say where the ladder stopped.
#   (D) The baseline itself != 43 ⇒ ★★★ STOP. `43` is at n=1. A baseline that does not
#       reproduce is a far more important result than any ablation, and every ablation below
#       it would be built on one boot.
#
# ## ⚠ AN ARM YOU SET IS NOT AN ARM IN FORCE
#
#   The summary row below reads the arm from the DEVICE'S OWN EMISSIONS (`VAS-PUBLISH arm=`,
#   `OPERAND-JOIN arm=`, `PT-SWEEP ... ran=`) as well as from the script's record of intent.
#   Until master `91f8b34b` the "EVERY RELAXATION THAT WAS ON" block printed `[]` eleven times
#   on a GREEN run — indistinguishable from "nothing was relaxed". For THIS rung that block IS
#   the experiment, so it is asserted non-empty here rather than merely printed.
set -uo pipefail

LABEL=${1:-}
[ -n "$LABEL" ] || { echo "usage: $0 <label> [VAR=VALUE ...]" >&2; exit 64; }
shift

REPO=${KAYFABE_REPO:-/workspace/kayfabe_w298}
export KAYFABE_REPO="$REPO"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w298}
export KAYFABE_TAG="w298${LABEL}"
export GQ_TIMEOUT=${GQ_TIMEOUT:-900}

# ---------------------------------------------------------------------------------------
# The overrides. ⊘ ONE, unless explicitly told otherwise.
# ---------------------------------------------------------------------------------------
NOVR=0
OVR_DESC="(none — BASELINE, the w297 arming byte for byte)"
DESCS=""
for kv in "$@"; do
  case "$kv" in
    *=*) ;;
    *) echo "★ not a VAR=VALUE override: [$kv]" >&2; exit 64 ;;
  esac
  k=${kv%%=*}; v=${kv#*=}
  NOVR=$((NOVR + 1))
  DESCS="$DESCS $k=$v"
  if [ "$k" = "KAYFABE_VAS_PUBLISH" ]; then
    # ⚠ POSITIONAL in w290p_run.sh, so it must travel as W298_ARM or it is silently ignored —
    #   the exact failure mode this whole rung exists to make impossible.
    export W298_ARM="$v"
  else
    export "$k=$v"
  fi
done
[ "$NOVR" -eq 0 ] || OVR_DESC="$DESCS"
if [ "$NOVR" -gt 1 ] && [ "${W298_MULTI:-0}" != "1" ]; then
  echo "★ $NOVR overrides in one boot makes the outcome unattributable. Set W298_MULTI=1 if" >&2
  echo "  that is genuinely what you want." >&2
  exit 64
fi

OUT=/workspace/${KAYFABE_TAG}.log
S=/workspace/w298_summary.txt
Q=/workspace/bench/run_${KAYFABE_TAG}_qemu.log
P=/workspace/bench/run_${KAYFABE_TAG}_probe.log
D=/workspace/bench/run_${KAYFABE_TAG}_hostdmesg.log

# ★★★ START MARKER — "exists but has no terminator" must be detectable at all. `143` (the job
#   was killed) and `124` (the LAUNCHER expired while the job ran on fine) arrive as the same
#   word otherwise, and this tree has read a zero-byte file as "still in flight" three times.
echo "=== W298 ABLATION START label=$LABEL overrides=[$OVR_DESC] $(date -Is) pid=$$ ===" \
     | tee -a "$S"

"$REPO/scripts/bench/w297_cup3.sh"
ARC=$?

# ---------------------------------------------------------------------------------------
# THE SUMMARY ROW. One line per boot, appended to a file that survives the next boot.
# ---------------------------------------------------------------------------------------
# ⚠ `grep -c X f || echo UNMEASURED` prints BOTH `0` and the fallback — `grep -c` prints 0 AND
#   exits 1. Existence is therefore its own test here, before any count, so "0 in a file that
#   exists" and "no file at all" are DIFFERENT WORDS.
gc() { # gc <pattern> <file>  -> count, or the unmeasured token
  if [ -e "$2" ]; then grep -c -- "$1" "$2" 2>/dev/null || true; else echo "⊘UNMEASURED"; fi
}

VAL=$(grep -oE '^CUP3_VAL=[0-9]+' "$P" 2>/dev/null | tail -1)
VAL=${VAL:-⊘ABSENT-UNMEASURED}
RC=$(grep -oE '^CUP3_RC=[0-9]+' "$P" 2>/dev/null | tail -1)
FIRSTX=$(grep -E '^    ✘ ' "$P" 2>/dev/null | head -1 | sed 's/^ *//')
LADDER_OK=$(grep -cE '^    ✔ ' "$P" 2>/dev/null || true)
XID=$(gc Xid "$D")
XIDLINE=$(grep -oE 'Xid[^,]*, pid=[0-9]*[^,]*' "$D" 2>/dev/null | head -1)
FAULTLVL=$(grep -oE 'FAULT_(PDE|PTE)[0-9]*' "$D" 2>/dev/null | sort -u | tr '\n' ' ')
FENG=$(grep -oE 'ENGINE [A-Z0-9_]+|HUBCLIENT_[A-Z0-9_]+|ACCESS_TYPE_[A-Z_]+' "$D" 2>/dev/null | sort -u | tr '\n' ' ')
FVA=$(grep -oE 'faulted @ 0x[0-9a-f_]+' "$D" 2>/dev/null | tail -1 | grep -oE '0x[0-9a-f_]+')
HROWS=$(grep -oE 'host_rows=[0-9]+ of [0-9]+' "$Q" 2>/dev/null | sort -u | tr '\n' ';')
INFORCE=$( { grep -oE 'VAS-PUBLISH arm=[a-z]+' "$Q" 2>/dev/null | head -1
             grep -oE 'OPERAND-JOIN arm=[a-z]+' "$Q" 2>/dev/null | head -1
             grep -oE 'PT-SWEEP tasks=[0-9]+ skipped=[0-9]+ ran=[0-9]+' "$Q" 2>/dev/null | tail -1
           } | tr '\n' ' ')
DOOR=$(grep -o 'by engine: .*' "$Q" 2>/dev/null | tail -1)
JIT=$(grep -oE 'CUP3_JIT_PRESENT=(yes|no)' "$P" 2>/dev/null | tail -1)
UNSERV=$(grep -ho 'unserviced fn 76 cmd 0x[0-9a-f]*' "$Q" 2>/dev/null | sort -u | wc -l)
REFUSED=$(grep -oE '⊘REFUSED `[^`]*`' "$Q" 2>/dev/null | sort | uniq -c | tr '\n' ' ')
# ★ THE RELAXATION REPORT IS THE EXPERIMENT — assert it is non-empty, do not merely print it.
RELAX=$(grep -cE '^    KAYFABE_[A-Z_]+ = \[KAYFABE_[A-Z_]+=[a-z]+\]' "$OUT" 2>/dev/null || true)

{
echo "--------------------------------------------------------------------------------"
echo "W298 ROW  label=$LABEL  overrides=[$OVR_DESC]  arc=$ARC  $(date -Is)"
echo "  ^CUP3_VAL      = $VAL          ^CUP3_RC = ${RC:-⊘ABSENT}"
echo "  JIT precond    = ${JIT:-⊘ABSENT-UNMEASURED}"
echo "  ladder ✔count  = ${LADDER_OK}   first ✘ = ${FIRSTX:-<none>}"
echo "  Xid count      = $XID   [${XIDLINE:-no Xid line}]"
echo "  fault level    = ${FAULTLVL:-⊘none printed}"
echo "  fault engine   = ${FENG:-⊘none printed}"
echo "  fault VA       = ${FVA:-⊘none}"
echo "  host_rows      = ${HROWS:-⊘ABSENT-UNMEASURED}"
echo "  ARM IN FORCE   = ${INFORCE:-⊘ DEVICE EMITTED NO ARM LINE — the arm is UNWITNESSED}"
echo "  doorbells      = ${DOOR:-⊘ABSENT-UNMEASURED}"
echo "  unserviced ids = $UNSERV"
echo "  refused pins   = ${REFUSED:-none}"
echo "  relaxation report rows = $RELAX  (MUST be 12; 0 means the block is VACUOUS and every"
echo "                                    'was on' reading below it is unsourced)"
echo "  logs: $OUT  Q=$(stat -c%s "$Q" 2>/dev/null || echo MISSING)b  P=$(stat -c%s "$P" 2>/dev/null || echo MISSING)b"
echo "=== W298 ROW EXIT label=$LABEL rc=$ARC $(date -Is) ==="
} | tee -a "$S"

exit "$ARC"
