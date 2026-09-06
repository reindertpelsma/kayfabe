#!/usr/bin/env bash
# ★★★★★ w384 — WHAT ONE DOORBELL COSTS THE SUBMITTING THREAD, AS A DIFFERENTIAL.
#
#   usage: KAYFABE_REPO=<tree> CARGO_TARGET_DIR=<dir> scripts/bench/w384_doorbell_latency.sh [tag]
#
# ## ★★★ WHY THIS EXISTS
#
# `[measured, LLM boot, w383 lane]` publication runs 60-71 ms per doorbell INLINE on the vCPU
# thread — `TRAPWITNESS off_trap_claims=0 inline_exceptions=61865 worst_trap=1750538us`.
# Thousands of doorbells at that price is why the LLM rung dies on a harness timeout with
# doorbells still being served, and NOTHING in this tree measured it in under twenty minutes.
# `LLM_TOKENS` — the async lane's only feedback — needs a full boot plus a model load and can
# come back UNMEASURED for reasons unrelated to the change under test.
#
# This is that measurement in seconds, from an ordinary unprivileged raw client.
#
# ## ★★★ THE NATIVE ARM IS NOT A SECOND OPINION, IT IS THE UNIT
#
#   A GUEST-ONLY NUMBER IS NOT A MEASUREMENT: A MILLISECOND IS ONLY BIG NEXT TO SOMETHING.
#
# The native arm runs FIRST, with no reference, and prints `DBL_CALIBRATION_NATIVE_P50_US`.
# This script reads that number off the native log and feeds it to the guest arm as
# `--doorbell-latency-native-us`, so the guest is graded against bare metal measured by the
# SAME binary (md5 printed on both arms) on the SAME box in the same minutes. ⊘ There is no
# baked millisecond threshold anywhere in this lane, deliberately: a capture-derived constant
# expires as the box changes and nobody notices.
#
# ## ★★★ PRE-REGISTERED, BEFORE THE RUN — every outcome, so none reads as the favourable one
#   (A) native PASS + guest FAIL   => ★★★★★ THE INTENDED SHAPE. A five-second gate for the
#                                     async lane, attributable because the control passed.
#   (B) native PASS + guest PASS   => ⚠⚠ A FINDING, NOT A GREEN. It would mean the 60-71 ms
#                                     is not on the raw client's doorbell path and the LLM's
#                                     cost has been mis-attributed. REPORT IT LOUDLY; do not
#                                     tune the rung until it goes red.
#   (C) native FAIL                => ⊘ the PROBE or the HOST, never the guest. Fix first.
#   (D) either arm NOTRUN          => ⊘ UNINTERPRETABLE — a control failed, so the numbers are
#                                     over submissions that carried no work. NOT a failure value.
#   (E) no binary                  => ⊘ UNMEASURED_NO_BINARY — attributable to the BUILD.
#   (F) guest never answered       => ⊘ UNMEASURED_NO_GUEST — attributable to the BOOT. ★ A
#                                     DIFFERENT fact from (D) and it does not share a verdict.
#   (G) no calibration number      => ⊘ UNMEASURED_NO_FLOOR — the native arm ran but printed
#                                     no `DBL_CALIBRATION_NATIVE_P50_US`, so the guest arm has
#                                     nothing to be a multiple of. Grading it anyway would be
#                                     inventing the threshold this lane exists not to invent.
#
# ⊘ **GRADED ON PRINTED LINES, NEVER ON EXIT STATUS.** A timeout kills the process and its
#   status says nothing about how far it got; a pipe makes `$?` the pager's.
# ⊘ **`dmesg -C` IS NOT CALLED.** A watermark is taken instead: clearing the host ring buffer
#   on a shared box destroys a concurrent lane's evidence.
# ⚠ **The host Xid watermark is not zero and must never be read absolutely** — `--missing-page-
#   fault` provokes a real `Xid 31` by design and other lanes run it. Bracket, never count.
set -uo pipefail

REPO=${KAYFABE_REPO:-/root/kayfabe}
export KAYFABE_REPO="$REPO"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w384}
TAG=${1:-w384}
OUT=/workspace/${TAG}_doorbell_latency.log
# ★ THREE PROCESSES, not three loops inside one. The rung already repeats internally, but
#   `submit_ms` has been measured at 9.1x across three CONSECUTIVE BOOTS of ONE build, and no
#   number of repetitions inside a process can see that. Both layers are needed.
NATIVE_RUNS=${W384_NATIVE_RUNS:-3}
RUNG_ARGS=${W384_RUNG_ARGS:---doorbell-latency}

BIN=""
for c in "$CARGO_TARGET_DIR"/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder \
         "$CARGO_TARGET_DIR"/release/kayfabe-rm-ladder \
         "$REPO"/target/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder; do
  [ -x "$c" ] && BIN="$c" && break
done

{
echo "=== ★★★★★ W384 DOORBELL-LATENCY DIFFERENTIAL START $(date -Is) ==="
echo "=== HEAD=[$(git -C "$REPO" rev-parse --short HEAD 2>/dev/null || echo '⊘ NO REPO — UNATTRIBUTABLE')]"
echo "=== DIRT=[$(git -C "$REPO" status --porcelain --untracked-files=no 2>/dev/null | head -3)]"
if [ -z "$BIN" ]; then
  echo "W384_OUTCOME=(E) ⊘ UNMEASURED_NO_BINARY — kayfabe-rm-ladder was never built"
  echo "    build: cargo build --release --target x86_64-unknown-linux-musl --bin kayfabe-rm-ladder"
  exit 0
fi
# ⊘ WHICH COPY RAN. The md5 is what lets the two arms be joined as THE SAME PROGRAM, which is
#   the entire content of the word "differential".
MD5=$(md5sum < "$BIN" | cut -d' ' -f1)
echo "=== BIN=$BIN md5=$MD5 $(stat -c %s "$BIN") bytes"
case "$(file -b "$BIN")" in
  *static*) echo "=== ★ STATIC — the same file will run inside the guest" ;;
  *) echo "=== ⊘⊘ NOT STATICALLY LINKED. A load failure inside the guest reports as 'no GPU',"
     echo "===    which is a completely different finding. Build the musl target."
     echo "W384_OUTCOME=(E) ⊘ UNMEASURED_NO_BINARY — dynamic binary, unusable in the guest"
     exit 0 ;;
esac
DMESG_MARK=$(sudo dmesg 2>/dev/null | wc -l)
echo "=== W384_DMESG_WATERMARK=$DMESG_MARK lines (⊘ a watermark, NOT a clear)"

################################################################################
echo ""
echo "################ ARM 1 — NATIVE, x$NATIVE_RUNS. THE CALIBRATION ################"
# ⊘ No `--doorbell-latency-native-us` here, on purpose: a calibration cannot be graded
#   against a floor derived from itself, and the rung says so in its own verdict.
NLOG=/workspace/bench/run_${TAG}_native.log
: > "$NLOG"
for i in $(seq 1 "$NATIVE_RUNS"); do
  echo "--- native run $i/$NATIVE_RUNS ---" | tee -a "$NLOG"
  # ⊘ NOT piped into anything: a pipe makes `$?` the pager's, and this repo has already had a
  #   harness print success over a workspace that never compiled.
  timeout 300 "$BIN" $RUNG_ARGS >>"$NLOG" 2>&1
  echo "W384_NATIVE_RUN${i}_RC=$?" >> "$NLOG"
done
grep -a '^DBL_DIST\|^DBL_MEASURED_P50_US\|^DBL_CALIBRATION\|^RUNG_doorbell_latency\|^RUNGCTL_doorbell_latency\|^DBL_ROLE\|^DBL_CFG\|^DBL_WITNESS' "$NLOG" | sed 's/^/    /'

# ★★★ THE FLOOR. Taken as the WORST (largest) of the native calibrations, not the best.
#   A floor picked from the fastest run would make the gate tighter than the host itself can
#   reliably deliver, and the first thing that goes red would be host jitter wearing the
#   guest's name.
FLOOR=$(grep -ah '^DBL_CALIBRATION_NATIVE_P50_US=' "$NLOG" | sed 's/.*=//' | sort -g | tail -1)
NPASS=$(grep -ac '^RUNG_doorbell_latency=PASS' "$NLOG")
NSEEN=$(grep -ac '^RUNG_doorbell_latency=' "$NLOG")
echo ""
echo "    W384_NATIVE_ROW runs=$NSEEN pass=$NPASS floor_us=[${FLOOR:-NONE}] (⊘ the WORST of the"
echo "                    native calibrations, never the best — see the header)"

if [ "$NSEEN" -eq 0 ]; then
  echo "W384_OUTCOME=(D) ⊘ UNINTERPRETABLE — the native arm printed no verdict line at all"
  exit 0
fi
if [ "$NPASS" -eq 0 ]; then
  echo "W384_OUTCOME=(C) ⊘ NATIVE FAILED — the PROBE or the HOST, never the guest. Fix that first."
  exit 0
fi
if [ -z "$FLOOR" ]; then
  echo "W384_OUTCOME=(G) ⊘ UNMEASURED_NO_FLOOR — native passed but printed no calibration number,"
  echo "    so the guest arm has nothing to be a multiple of. ⊘ Grading it against an invented"
  echo "    threshold is precisely what this lane exists not to do."
  exit 0
fi

################################################################################
echo ""
echo "################ ARM 2 — MODE-2 GUEST, graded against ${FLOOR}us ################"
# ⚠ `KAYFABE_CE_EXECUTOR=local` is the arm under test and is set EXPLICITLY, never left to a
#   default: with `local` the shell's CPU copy engine serves every CE doorbell. With `host`
#   the same doorbells go to the forwarding plane and this would be a different measurement
#   under the same name.
KAYFABE_TAG=${TAG}_guest \
KAYFABE_CE_EXECUTOR=local \
GQ_TIMEOUT=${GQ_TIMEOUT:-900} \
KAYFABE_W384_BIN="$BIN" \
KAYFABE_W384_FLOOR_US="$FLOOR" \
KAYFABE_W384_RUNS="${W384_GUEST_RUNS:-3}" \
KAYFABE_W384_BISECT="${W384_BISECT:-}" \
KAYFABE_W384_MISSING_PAGE="${W384_MISSING_PAGE:-1}" \
POST_CAPTURE_HOOK="$REPO/scripts/bench/w384_hook.sh" \
  bash "$REPO/scripts/bench/w290p_run.sh" drain
echo "=== guest arm inner rc=$? (⊘ NOT the grade — see the header) ==="

# ⊘ The guest row lives in `boot_capture.sh`'s PROBE log, because that is where a
#   POST_CAPTURE_HOOK's output is written — NOT in `w290p_run.sh`'s own `/workspace/<tag>.log`.
#   Reading the wrong one of those two is how a table printed "no row" over a graded arm.
GOUT=/workspace/bench/run_${TAG}_guest_probe.log

################################################################################
echo ""
echo "================================================================================"
echo "=== ★★★★★ THE DIFFERENTIAL — the deliverable"
echo "================================================================================"
printf '    %-10s %-9s %s\n' ARM VERDICT DISTRIBUTION
# ⊘ EVERY run's row, not the last one. Each arm runs the binary three times and a `tail -1`
#   would silently report whichever process happened to finish last — which is exactly how a
#   bimodal result reads as a clean one. The counts below are the grade; the rows are evidence.
nver=$(grep -ah '^RUNG_doorbell_latency=' "$NLOG" | sed 's/.*=//' | sort | uniq -c | tr -s ' ' | tr '\n' ' ')
gver=$(grep -ah 'RUNG_doorbell_latency=' "$GOUT" 2>/dev/null | sed 's/.*=//' | sort | uniq -c | tr -s ' ' | tr '\n' ' ')
printf '    %-10s %s\n' native "[${nver:-NONE}]"
grep -ah 'DBL_DIST arm=submit rep=POOLED' "$NLOG" | sed 's/^/      /'
printf '    %-10s %s\n' guest "[${gver:-NONE}]"
grep -ah 'DBL_DIST arm=submit rep=POOLED' "$GOUT" 2>/dev/null | sed 's/^/      /'
echo "    DBL_RATIO_X (guest, one per run): [$(grep -ah '^DBL_RATIO_X=' "$GOUT" 2>/dev/null | sed 's/.*=//' | tr '\n' ' ')]"
# ★ The hook already reached a verdict over ALL of its runs and printed the letter. Re-deriving
#   it here from a single line would be a SECOND source of truth beside a complete value.
GOUTCOME=$(grep -ah 'W384_GUEST_OUTCOME=' "$GOUT" 2>/dev/null | tail -1 | sed 's/.*W384_GUEST_OUTCOME=//')
echo "    guest hook's own verdict: ${GOUTCOME:-⊘ NONE — the hook never reached one}"
# The pairing rule below needs one word per arm; take the MAJORITY outcome, and say so.
nver1=$(grep -ah '^RUNG_doorbell_latency=' "$NLOG" | sed 's/.*=//' | sort | uniq -c | sort -rn | head -1 | awk '{print $2}')
gver1=$(grep -ah 'RUNG_doorbell_latency=' "$GOUT" 2>/dev/null | sed 's/.*=//' | sort | uniq -c | sort -rn | head -1 | awk '{print $2}')
echo ""
echo "=== ★ THE OTHER TWO ARMS — printed, UNGRADED, and they are what makes a green readable"
for a in bare freshmap; do
  printf '    %-10s native: %s\n' "$a" "$(grep -ah "DBL_DIST arm=$a" "$NLOG" | tail -1)"
  printf '    %-10s guest : %s\n' "$a" "$(grep -ah "DBL_DIST arm=$a" "$GOUT" 2>/dev/null | tail -1)"
done

echo ""
echo "=== ★★★ THE PAIRING RULE, applied"
case "${nver1:-NONE}/${gver1:-NONE}" in
  PASS/FAIL)
    echo "    W384_OUTCOME=(A) ★★★★★ THE INTENDED SHAPE — native PASS, guest FAIL, control passed"
    echo "        on both. This is a five-second gate for w383-doorbell-async." ;;
  PASS/PASS)
    echo "    W384_OUTCOME=(B) ⚠⚠ A FINDING, NOT A GREEN. The guest is within ${FLOOR}us x the"
    echo "        gate multiple, which means the 60-71 ms inline publication is NOT on the raw"
    echo "        client's doorbell path and the LLM's cost has been mis-attributed."
    echo "        ⇒ Read \`arm=freshmap\` against \`arm=submit\` on the GUEST row above: if"
    echo "        freshmap is the expensive one, the cost is in PUBLISHING NEW ROWS and this"
    echo "        loop had nothing left to publish. DO NOT tune the rung until it goes red." ;;
  PASS/NOTRUN|PASS/NONE)
    echo "    W384_OUTCOME=(D) ⊘ UNINTERPRETABLE — the guest arm never reached its control, or"
    echo "        printed no verdict. NOT a failure value, and NOT the same as (F)." ;;
  *)
    echo "    W384_OUTCOME=(C) ⊘ native did not pass — the PROBE or the HOST, never the guest." ;;
esac
echo ""
echo "=== the host's own word since the watermark (⊘ bracketed, never counted absolutely)"
sudo dmesg 2>/dev/null | tail -n +"$((DMESG_MARK + 1))" | grep -iE 'xid|nvrm' | tail -10 | sed 's/^/    /'
echo "=== W384 DIFFERENTIAL EXIT at $(date -Is) ==="
} 2>&1 | tee "$OUT"
