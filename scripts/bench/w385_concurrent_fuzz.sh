#!/usr/bin/env bash
# ★★★★★ w385 — THE CONCURRENT FUZZ, AS A LADDER OF ARMS, GRADED AS A TABLE.
#
# ## WHY A LADDER AND NOT ONE RUN
#
# A single green fuzz run says almost nothing: it sampled ONE schedule of ONE seed at ONE
# thread count. So this walks the thread count up and reports the budget each rung actually
# exhausted, and it runs the SAME seed twice at the same width — because *"the seed replays"*
# is a claim the rung makes about itself, and an unchecked claim about an instrument is the
# thing this tree has paid for most often.
#
#   usage: KAYFABE_REPO=<tree> CARGO_TARGET_DIR=<dir> scripts/bench/w385_concurrent_fuzz.sh [tag]
#          W385_SEED=<n>      pin the seed for every arm (default: each arm draws and prints one)
#          W385_ARMS="..."    override the ladder, as `T:I` pairs
#
# ## ★★★ PRE-REGISTERED, BEFORE THE RUN — every outcome, so none reads as the favourable one
#   (A) every arm PASS                       => no invariant broke under any sampled schedule.
#                                               ⚠ A BUDGET, NOT A PROOF.
#   (B) any arm FAIL, control PASS           => ★★★★★ A REAL RED, and the seed on that arm's
#                                               line replays its decision sequence. Worth far
#                                               more than a green.
#   (C) any arm NOTRUN with RUNGCTL FAIL     => ⊘ UNINTERPRETABLE — the rung measured its own
#                                               harness, not the system. NOT a failure value.
#   (D) any arm NOTRUN with FUZZ_REASON=NO_CONCURRENCY_OBSERVED
#                                            => ⊘ the arm sampled no overlap at all. Also not
#                                               a failure value, and NOT a pass either.
#   (E) no `RUNG_concurrent_fuzz=` line      => ⊘ UNMEASURED. The binary ran and said nothing.
#   (F) no binary                            => ⊘ UNMEASURED_NO_BINARY — attributable to the
#                                               BUILD, not to the GPU.
#   (G) no /dev/nvidiactl                    => ⊘ UNMEASURED_NO_DEVICE — attributable to the
#                                               HOST. ★ Distinct from (E) and from (F): "there
#                                               was no GPU", "there was no program" and "the
#                                               program said nothing" are three different
#                                               facts and must not share a verdict.
#   (H) FUZZ_REASON=DEADLOCK/WATCHDOG        => ★★ a nontermination that FAILED BY NAME
#                                               instead of wedging. Graded FAIL, on purpose.
#
# ⊘ **GRADED ON PRINTED LINES, NEVER ON EXIT STATUS.** The rung's watchdog ends the process
#   with `exit(3)` when it fires, so the status is a statement about the watchdog and not
#   about the measurement. Every field below is `sed`-ed out of the log.
# ⊘ **`sed`s EXACTLY `RUNG_concurrent_fuzz=`.** w377 printed prose while its grader looked
#   for a name nothing emitted, and a 3/3 passing run graded as UNMEASURED.
set -uo pipefail

REPO=${KAYFABE_REPO:-/root/kayfabe}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w385}
TAG=${1:-w385}
BIN=${W385_BIN:-$CARGO_TARGET_DIR/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder}
OUTDIR=${W385_OUTDIR:-/workspace/bench}
OUT=$OUTDIR/${TAG}_concurrent_fuzz.log
# T:I — the ladder. Widths first, then depth, then the replay pair at the widest width.
# ⊘ `T:I` — T is the UNPINNED width and the per-core width; the rung derives its own
# over-subscribed width (3 x cores) from `--fuzz-cores`. The default row is the bench guest's
# `-smp 3`, because that is the topology the product presents.
ARMS=${W385_ARMS:-"3:128 3:512 6:256 12:128"}
CORES=${W385_CORES:-3}
TMO=${W385_TIMEOUT:-1200}
mkdir -p "$OUTDIR"

{
echo "=== ★★★★★ W385 CONCURRENT FUZZ START $(date -Is) on $(hostname) ==="
echo "=== HEAD=[$(git -C "$REPO" rev-parse --short HEAD 2>/dev/null || echo '⊘ NO REPO — UNATTRIBUTABLE')]"
echo "=== DIRT=[$(git -C "$REPO" status --porcelain --untracked-files=no 2>/dev/null | head -3)]"
echo "=== DRIVER=[$(head -1 /proc/driver/nvidia/version 2>/dev/null || echo none)]"
echo "=== CORES=$(nproc)"

# (G) the device, before the binary: a missing GPU is the HOST's fact and must not be
#     reported as ours.
if [ ! -c /dev/nvidiactl ]; then
  echo "W385_OUTCOME=(G) ⊘ UNMEASURED_NO_DEVICE — /dev/nvidiactl absent; this is the HOST"
  echo "=== W385 EXIT at $(date -Is) ==="
  exit 0
fi
# (F) the binary.
if [ ! -x "$BIN" ]; then
  echo "W385_OUTCOME=(F) ⊘ UNMEASURED_NO_BINARY — no $BIN"
  echo "    build: cargo build --release --target x86_64-unknown-linux-musl --bin kayfabe-rm-ladder"
  echo "=== W385 EXIT at $(date -Is) ==="
  exit 0
fi
echo "=== BIN=$BIN md5=$(md5sum < "$BIN" | cut -d' ' -f1) $(stat -c %s "$BIN") bytes"

# ⊘ Every arm gets its own log, so a wedged arm's evidence is not the next arm's problem and
#   a per-arm grade is readable without splitting one file.
declare -a ROWS=()
run_arm() {   # $1 label, $2 threads, $3 iters, $4 seed-or-empty, $5 extra args
  local label=$1 t=$2 i=$3 seed=$4 extra=$5
  local log=$OUTDIR/run_${TAG}_${label}.log
  local seedarg=()
  [ -n "$seed" ] && seedarg=(--seed "$seed")
  echo ""
  echo "################ ARM $label — T=$t I=$i ${seed:+seed=$seed} $extra ################"
  # shellcheck disable=SC2086
  timeout -k 15 "$TMO" "$BIN" --gpu 0 --concurrent-fuzz \
      --fuzz-threads "$t" --fuzz-iters "$i" --fuzz-cores "$CORES" "${seedarg[@]}" $extra > "$log" 2>&1
  local rc=$?
  echo "    inner rc=$rc  (⊘ NOT the grade — the rung's watchdog exits 3 by design)"
  tail -40 "$log"
  # ── the grade, out of PRINTED LINES only ──────────────────────────────────────────────
  local v ctl reason ops overlap viol useed
  v=$(sed -n 's/^RUNG_concurrent_fuzz=//p'    "$log" | tail -1)
  ctl=$(sed -n 's/^RUNGCTL_concurrent_fuzz=//p' "$log" | tail -1)
  reason=$(sed -n 's/^FUZZ_REASON=//p'        "$log" | tail -1)
  ops=$(sed -n 's/^FUZZ_OPS=//p'              "$log" | tail -1)
  overlap=$(sed -n 's/^FUZZ_OVERLAP_PAIRS=//p' "$log" | tail -1)
  viol=$(sed -n 's/^FUZZ_VIOLATIONS=//p'      "$log" | tail -1)
  useed=$(sed -n 's/^FUZZ_SEED=//p'           "$log" | tail -1)
  ROWS+=("$(printf '%-10s %-4s %-5s %-20s %-8s %-10s %-10s %-6s %s' \
      "$label" "$t" "$i" "${useed:-⊘none}" "${v:-⊘NONE}" "${ctl:-⊘NONE}" \
      "${ops:-⊘}" "${viol:-⊘}" "${reason:-⊘none} overlap=${overlap:-⊘}")")
  # every named invariant that fired, verbatim, with its arm
  sed -n 's/^FUZZ_VIOLATION=/    ★ '"$label"' fired: /p' "$log"
  # ★ EVERY ARM'"'"'S OWN LINE, so "pinning changed the answer" is visible per run and not
  #   only in the aggregate the rung prints for itself.
  sed -n 's/^FUZZ_ARM=/    · '"$label"'/p' "$log"
}

for spec in $ARMS; do
  t=${spec%%:*}; i=${spec##*:}
  run_arm "t${t}i${i}" "$t" "$i" "${W385_SEED:-}" ""
done

# ★★★ THE REPLAY PAIR. The rung claims `--seed` replays its decision sequence; that claim is
# checked here rather than believed. ⊘ It checks the DECISIONS, not the schedule: the two
# runs must agree on the per-verb census, and are NOT required to agree on timing.
REPLAY_SEED=${W385_REPLAY_SEED:-0x5EED0385}
run_arm "replay-a" 8 96 "$REPLAY_SEED" ""
run_arm "replay-b" 8 96 "$REPLAY_SEED" ""

# ═══════════════════════════════════════════════════════════════════════════════════════
# ★★★ THE WATCHDOG'"'"'S OWN NEGATIVE CONTROL — because a watchdog that has never fired is
# not a watchdog, it is a comment. This arm gives the rung an IMPOSSIBLE deadline and asserts
# it FAILS BY NAME rather than hanging: a `RUNG_concurrent_fuzz=FAIL` line, a
# `FUZZ_REASON=DEADLOCK/WATCHDOG` line, and a per-worker dump.
# ⊘ It is a control on the INSTRUMENT and is graded separately; it is not one of the arms
#   above and must never be folded into their verdict.
echo ""
echo "################ CONTROL — the WATCHDOG, provoked on purpose ################"
WLOG=$OUTDIR/run_${TAG}_watchdog.log
timeout -k 15 180 "$BIN" --gpu 0 --concurrent-fuzz --fuzz-threads 8 --fuzz-iters 100000 \
    --fuzz-cores "$CORES" --fuzz-deadline 5 --seed 0x385 > "$WLOG" 2>&1
echo "    inner rc=$?  (⊘ 3 is what the watchdog exits with, by design)"
WV=$(sed -n 's/^RUNG_concurrent_fuzz=//p' "$WLOG" | tail -1)
WR=$(sed -n 's/^FUZZ_REASON=//p' "$WLOG" | tail -1)
WD=$(grep -c "last op = " "$WLOG")
if [ "$WV" = "FAIL" ] && [ "$WR" = "DEADLOCK/WATCHDOG" ] && [ "$WD" -gt 0 ]; then
  echo "    W385_WATCHDOG=PASS — it fired, printed FAIL by name, and dumped $WD worker rows"
else
  echo "    W385_WATCHDOG=FAIL — ⊘ the watchdog did NOT fail by name."
  echo "      verdict=[${WV:-none}] reason=[${WR:-none}] worker rows=[$WD]"
  echo "      ⚠ Every green above is then held up by an instrument that has never been shown"
  echo "        to work, and a hang would wedge instead of failing."
fi
grep -a "last op = " "$WLOG" | head -10

echo ""
echo "================================================================================"
echo "=== ★★★★★ THE W385 TABLE — one row per arm, graded on printed lines"
echo "================================================================================"
printf '    %-10s %-4s %-5s %-20s %-8s %-10s %-10s %-6s %s\n' \
    ARM T I SEED VERDICT CONTROL OPS VIOL REASON
for r in "${ROWS[@]}"; do echo "    $r"; done

echo ""
echo "=== ★★ THE REPLAY CHECK — same seed, same decisions?"
A=$OUTDIR/run_${TAG}_replay-a.log; B=$OUTDIR/run_${TAG}_replay-b.log
verbs_of() { grep -a 'W385 fuzz' "$1" 2>/dev/null | grep -a 'verbs' | sed 's/.*verbs  *= *//' | tail -1; }
VA=$(verbs_of "$A")
VB=$(verbs_of "$B")
if [ -z "$VA" ] || [ -z "$VB" ]; then
  echo "    W385_REPLAY=⊘ UNMEASURED — one of the replay arms printed no verb census"
elif [ "$VA" = "$VB" ]; then
  echo "    W385_REPLAY=SAME — both arms drew the same verbs: [$VA]"
else
  echo "    W385_REPLAY=DIFFERENT — ⊘ the seed does NOT pin the verb census."
  echo "      a: [$VA]"
  echo "      b: [$VB]"
  echo "      ⚠ Expected when a verb is SKIPPED because slot state differed, which timing can"
  echo "        change. The DECISION STREAM is still pinned; the CENSUS is a weaker check and"
  echo "        a difference here is not by itself a defect."
fi

echo ""
echo "=== ★★★ THE READING RULE, applied"
echo "    * every arm PASS            ⇒ (A) no invariant broke under any schedule SAMPLED."
echo "                                   ⚠ Absence of a red is not absence of a race."
echo "    * any arm FAIL + control PASS ⇒ (B) ★★★★★ A REAL RED. Replay it with its SEED."
echo "    * any NOTRUN + control FAIL ⇒ (C) ⊘ the RUNG is broken, not the system."
echo "    * NOTRUN + NO_CONCURRENCY_OBSERVED ⇒ (D) ⊘ nothing was sampled. Not a pass."
echo "    * FUZZ_REASON=DEADLOCK/WATCHDOG ⇒ (H) ★★ a hang that failed BY NAME."
echo "=== W385 EXIT at $(date -Is) ==="
} 2>&1 | tee "$OUT"
