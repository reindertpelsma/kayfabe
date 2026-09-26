#!/usr/bin/env bash
# ★★★★★ w277 — **WHY 255 BINDINGS STRADDLE**, and the THIRD OUTCOME `w276` could not name.
#
# ⊘⊘ **THE DEVICE'S BEHAVIOUR IS UNCHANGED.** Every change on this rung is an INSTRUMENT: the
#    refusal carries more fields, the settlement counts a loss it used to swallow, and the
#    address table prints its own coverage. Nothing binds differently, nothing is relaxed, no
#    policy moved. ⇒ **`CUP2_RC=124` with the fault unmoved is the EXPECTED result**, and it is
#    pre-registered as such rather than discovered afterwards. A rung whose headline arm is
#    "the wall did not move" must say so BEFORE it runs, or the null reads as a disappointment
#    instead of as the control it is.
#
# ONE variable, unchanged from `w276`: `KAYFABE_PT_SWEEP`. Everything else is `w271_pin`'s
# arming byte for byte, so every carried counter stays directly comparable.
#
#   arm   FB_JOIN  GUEST_RING  GUEST_PUSHBUF  PT_WITNESS_EXEC  GUEST_SEMA  GR_ROUTE     GUEST_OPERAND  PT_SWEEP
#   off   shared   ring        pin            on               pin         passthrough  pin            off
#   on    shared   ring        pin            on               pin         passthrough  pin            ON
#
# ## WHAT IS BEING MEASURED, in order of importance
#
#  1. ★★★★★ **THE STRADDLE CENSUS — what actually differs between the two shapes.**
#     `[measured, w276b_on, read from traces/boots/w276/run_w276b_on_qemu.log.gz]` the sweep
#     printed `refusals=255 by_kind={"StraddlesLiveBinding": 255}` on **all 88** doorbells and
#     the payload was a virtual address and nothing else — a histogram with ONE bucket over
#     four causes that want opposite fixes. `straddles=` now reports a histogram over
#     `(shape, agreement, leaf level, leaf extent, live extent, live published)`.
#     ⚠ **READ `agreement` FIRST.** `SameMemory` = the live binding ALREADY resolves that VA to
#       the byte the leaf names ⇒ one fact at two granularities, the refusal is correct and
#       NEITHER source is wrong. `Contradicts` = two sources disagree about what backs a VA,
#       which is `w222`'s class and IS a bug in one of them.
#
#  2. ★★★★★ **THE THIRD OUTCOME — `shape_collisions`.** `w276` measured the faulting VA in no
#     bound list and no refused list. There is a third exit: `ReachShadow::settle`'s desired
#     set is keyed on the leaf's **VA alone**, so two leaves at one VA with different sizes
#     collided and the loser was dropped with no counter and no line. GA10x PD0 is a **dual
#     PDE** and our decoder follows both halves, so it is reachable by construction.
#     ⊘ RM's own walker makes the state impossible in a settled tree
#       (`ogkm-580: src/nvidia/src/libraries/mmu/mmu_walk.c:476-488, :1066-1092` —
#       `_mmuWalkResolveSubLevelConflicts` INVALIDATES the other sub-table over the VA range on
#       every map/unmap/sparsify) and the source **nowhere** says what hardware does if a VA is
#       valid in both. ⇒ a non-zero count here is an **anomaly**, not a tie to break.
#
#  3. ★★★★★ **TABLE-DESCRIBES — is the faulting VA BOUND?** A refusal list can never answer
#     that, and it is the reading `w276` could not choose between. The address table now prints
#     its own coalesced runs beside the guest's, and the join is offline, so an ASLR'd address
#     arriving after the hang can still be asked. ⊘ The device never learns the address.
#
#  4. ★★ **CARRIED, so the claims stay this boot's**: `swept_binds`, truncation, `FWD-RING`,
#     the SEMA write census, the Xid identity, `CUP2_RC`.
#
# ## ⊘⊘ WHAT THIS BOOT CANNOT PROVE, stated before it runs
#
#  - **It cannot move the wall, and is not built to.** See the banner above.
#  - **It cannot say which of two colliding declarations SHOULD win** — see 2; the ground truth
#    does not exist in the driver source, so this run reports and does not choose.
#  - **A `Contradicts` verdict does not name the guilty source.** It says two producers
#    disagree; which one is wrong needs their provenance, and only the leaf's LEVEL is carried.
#  - **The C measured a root walk INSUFFICIENT at this exact point** (`C: :8676-8688`) ⇒ a null
#    from a root sweep refutes nothing.
#  - Nothing here measures doorbell delivery, engine execution or throughput.
#  - One workload, one chip (GA106), one driver (580.159.04), one boot per arm.
#
# ★ START marker and EXIT line so "file exists but has no terminator" is detectable at all
#   (143 = the JOB was SIGTERMed; 124 = the LAUNCHER's ssh expired while the job ran on —
#   opposite meanings arriving as the same word).
PFX=${W277_TAG_PREFIX:-w277}
OUT=/workspace/${PFX}_run.log
exec >"$OUT" 2>&1
finish() { echo "=== W277 EXIT rc=$1 at $(date -Is) ==="; exit "$1"; }
echo "=== W277 START $(date -Is) pid=$$ ==="

export PATH=/root/.cargo/bin:$PATH
REPO=${KAYFABE_REPO:-/workspace/kayfabe_w277}
cd "$REPO" || finish 90
HEAD=$(git rev-parse HEAD)
echo "=== HEAD=$HEAD ==="
DIRT=$(git status --porcelain --untracked-files=no)
[ -z "$DIRT" ] || { echo "=== ★ TREE IS DIRTY ==="; echo "$DIRT"; finish 91; }

export CARGO_TARGET_DIR=/workspace/bench/cargo-target-w268
export KAYFABE_SHIM_FEATURES=host-isolates
echo "=== BUILD START $(date -Is) ==="
scripts/build_qom_shim.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build
BRC=$?
echo "=== BUILD RC=$BRC $(date -Is) ==="
[ $BRC -eq 0 ] || finish 92
# ⚠ From the SAME invocation: disk is the failure that reads as a code error (#181).
echo "=== BUILD ENOSPC/LLVM = $(grep -c 'No space left on device\|LLVM ERROR' "$OUT" 2>/dev/null) | df: $(df -h /workspace | tail -1) ==="

# ★★★ THE STAMP GATE, anchored to exactly 40 hex.
STAMP=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 2>/dev/null \
        | grep -oE 'kayfabe-rev:[0-9a-f]{40}' | head -1 | cut -d: -f2)
echo "=== STAMP=$STAMP HEAD=$HEAD ==="
[ "$STAMP" = "$HEAD" ] || { echo "=== ★★★ STAMP GATE FAIL: the binary is not this HEAD ==="; finish 93; }

# ⊘⊘ THE cap2b GUARD, carried from w271 and widened. No address this rung reasons about may be
#    a literal in the device: a sweep that RECALLS an address instead of DECODING one would
#    make every GUEST-DESCRIBES row read as evidence for a mechanism that is a constant.
CC_RC=0
NCAP=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 \
       | grep -c '0x2_04420000\|0x204420000\|0x2_04428000\|0x204428000\|0x2_0440fff0\|0x20440fff0')
echo "  ★★★ cap2b GUARD: fault/semaphore address literals in the binary = $NCAP (MUST be 0)"
[ "$NCAP" -eq 0 ] || { echo "  ★★★ AN ADDRESS IS A LITERAL — the pass recalls, it does not decode"; CC_RC=1; }
[ $CC_RC -eq 0 ] || finish 94

export KAYFABE_ISOLATES=real
export KAYFABE_CE_EXECUTOR=host
export NVKVM_RAM_BACKEND=memfd
export KAYFABE_GUEST_RAM=memfd
# ★ The w269 hook: launches cup2 DETACHED, waits for `totalMem=` (cup2's own last print before
#   `cuCtxCreate`), then runs the ptrace spin probe. It is what produces CUP2_RC.
export POST_CAPTURE_HOOK=$REPO/scripts/bench/cup2_hook_gdbspin.sh
export GQ_TIMEOUT=420
export BOOT_TIMEOUT=180
for f in scripts/bench/guest_spinprobe.c scripts/bench/cup2_hook_gdbspin.sh; do
  [ -f "$f" ] || { echo "=== ★★★ MISSING $f — the probe cannot run ==="; finish 96; }
done

# ⊘ EVERY per-arm variable is unset first and set only if this arm names it, so an arm can
#   never inherit the previous arm's arming.
boot() {
  local tag=$1 fbj=$2 ring=$3 pb=$4 wit=$5 sema=$6 route=$7 op=$8 swp=$9
  unset KAYFABE_FB_JOIN KAYFABE_GUEST_RING KAYFABE_GUEST_PUSHBUF KAYFABE_PT_WITNESS_EXEC \
        KAYFABE_GUEST_SEMA KAYFABE_GR_ROUTE KAYFABE_GUEST_OPERAND KAYFABE_PT_SWEEP
  [ "$fbj"   = "-" ] || export KAYFABE_FB_JOIN=$fbj
  [ "$ring"  = "-" ] || export KAYFABE_GUEST_RING=$ring
  [ "$pb"    = "-" ] || export KAYFABE_GUEST_PUSHBUF=$pb
  [ "$wit"   = "-" ] || export KAYFABE_PT_WITNESS_EXEC=$wit
  [ "$sema"  = "-" ] || export KAYFABE_GUEST_SEMA=$sema
  [ "$route" = "-" ] || export KAYFABE_GR_ROUTE=$route
  [ "$op"    = "-" ] || export KAYFABE_GUEST_OPERAND=$op
  [ "$swp"   = "-" ] || export KAYFABE_PT_SWEEP=$swp
  echo "=== BOOT $tag START $(date -Is) ==="
  echo "    KAYFABE_PT_SWEEP=[${KAYFABE_PT_SWEEP:-unset}] (THE variable) |" \
       "FB_JOIN=[${KAYFABE_FB_JOIN:-unset}] RING=[${KAYFABE_GUEST_RING:-unset}]" \
       "PUSHBUF=[${KAYFABE_GUEST_PUSHBUF:-unset}] WITNESS=[${KAYFABE_PT_WITNESS_EXEC:-unset}]" \
       "SEMA=[${KAYFABE_GUEST_SEMA:-unset}] ROUTE=[${KAYFABE_GR_ROUTE:-unset}]" \
       "OPERAND=[${KAYFABE_GUEST_OPERAND:-unset}]"
  timeout 1200 "$REPO/scripts/bench/boot_capture.sh" "$tag"
  echo "=== BOOT $tag RC=$? $(date -Is) ==="
  local Q=/workspace/bench/run_${tag}_qemu.log
  local P=/workspace/bench/run_${tag}_probe.log
  local D=/workspace/bench/run_${tag}_hostdmesg.log

  # ⊘ pgrep -x qemu-system-x86 (comm truncates at 15); NOT -f (it matches the asker).
  echo "--- post-boot liveness: pgrep -x qemu-system-x86 = [$(pgrep -x qemu-system-x86 | tr '\n' ' ')] ss2223 = [$(ss -tln 2>/dev/null | grep -c 2223)]"
  echo "--- ENOSPC_LLVM=$(grep -c 'No space left on device\|LLVM ERROR' "$Q" 2>/dev/null || echo '?')"
  echo "--- ARTEFACT SIZES: qemu=$(stat -c %s "$Q" 2>/dev/null || echo MISSING) probe=$(stat -c %s "$P" 2>/dev/null || echo MISSING) hostdmesg=$(stat -c %s "$D" 2>/dev/null || echo MISSING)"

  # ★★★★★ THE ARM ASSERTION FOR THIS RUNG'S ONE VARIABLE, read out of the device's own line.
  local wantswp='off'; [ "$swp" = "on" ] && wantswp='on'
  grep -q "kayfabe: PT-SWEEP arm=$wantswp" "$Q" 2>/dev/null \
    && echo "    ★ SWEEP-ARM ASSERTION: PASS (PT-SWEEP arm=$wantswp)" \
    || echo "    ★★★ SWEEP-ARM ASSERTION: FAIL — wanted PT-SWEEP arm=$wantswp. ⊘ THIS BOOT IS VOID."
  # ★★ The six carried arms, asserted so a mis-armed boot is not a data point that looks like one.
  for pair in "FB-JOIN arm=$fbj" "GUEST-RING arm=$ring" "GUEST-PUSHBUF arm=$pb" \
              "GUEST-SEMA arm=$sema" "GR-ROUTE arm=$route" "GUEST-OPERAND arm=$op"; do
    grep -q "kayfabe: $pair" "$Q" 2>/dev/null \
      && echo "    ★ CARRIED-ARM: PASS ($pair)" \
      || echo "    ★★★ CARRIED-ARM: FAIL — wanted '$pair'. ⊘ VOID for comparison."
  done
  local wantwit='DISARMED'; [ "$wit" = "on" ] && wantwit='ARMED'
  grep -q "EXEC-WITNESS $wantwit" "$Q" 2>/dev/null \
    && echo "    ★ WITNESS-ARM: PASS (EXEC-WITNESS $wantwit)" \
    || echo "    ★★★ WITNESS-ARM: FAIL — wanted EXEC-WITNESS $wantwit. ⊘ VOID."

  # ★ Comparability to the three boots this device is otherwise unchanged from.
  echo "--- COMPARABILITY (⊘ every baseline READ FROM ITS OWN ARTEFACT, never carried):"
  echo "    DOORBELL-XLATE = $(grep -c 'DOORBELL-XLATE' "$Q" 2>/dev/null)   (w271_pin: 88, w274_pin: 88, w275_pin: 88)"
  echo "    OPERAND-PIN    = $(grep -c 'OPERAND-PIN' "$Q" 2>/dev/null)   (w271_pin: 156, w274_pin: 223, w275_pin: 156)"
  echo "    doorbells      = [$(grep -o '[0-9]* doorbells served, [0-9]* of them forwarded' "$Q" 2>/dev/null | tail -1)]   (w275_pin: 196 / 12)"

  # =========================================================================================
  # ★★★★★ THIS RUNG'S HEADLINE — DID THE SWEEP RUN, AND DID THE RELAXATION DO ANYTHING?
  # =========================================================================================
  echo "--- ★★★★★ PT-SWEEP: line count = $(grep -c 'PT-SWEEP tasks=' "$Q" 2>/dev/null) (control wants 0, armed wants >0)"
  echo "    --- every distinct PT-SWEEP shape, with how often it occurred:"
  grep -oE 'PT-SWEEP tasks=[0-9]+ skipped=[0-9]+ ran=[0-9]+ truncated=[0-9]+ pages=[0-9]+ reasons=\{[^}]*\} → bound=[0-9]+ swept_binds=[0-9]+ unbound=[0-9]+ unwitnessed=[0-9]+ published=[0-9]+ faults=[0-9]+ reach_faults=[0-9]+ refusals=[0-9]+ first=[^ |]*' "$Q" 2>/dev/null \
    | sort | uniq -c | sort -rn | head -20 | sed 's/^/      /'
  # ⊘⊘ THE NUMBER THE OWNER'S RULING IS JUDGED ON. `swept_binds=0` on every row means the
  #    relaxation ran and published NOTHING the witness rule would not have — the gate moved
  #    and it did not matter. That is a RESULT, and it is invisible in a `bound=` total.
  echo "    --- ★★★★★ swept_binds, summed (the PRICE of the gate move, as a number):"
  awk 'match($0, /swept_binds=[0-9]+/) { s += substr($0, RSTART+13, RLENGTH-13); n++ }
       END { printf "      total=%d over %d PT-SWEEP rows (⊘ 0 with rows>0 ⇒ THE RELAXATION CHANGED NOTHING)\n", s+0, n+0 }' "$Q" 2>/dev/null
  echo "    --- ⚠ TRUNCATION (a truncated walk contributes NO leaves, not fewer — re-derive the budget):"
  awk 'match($0, /truncated=[0-9]+/) { t += substr($0, RSTART+10, RLENGTH-10) }
       match($0, /pages=[0-9]+/)     { p += substr($0, RSTART+6, RLENGTH-6); if (substr($0,RSTART+6,RLENGTH-6)+0 > mx) mx = substr($0,RSTART+6,RLENGTH-6)+0 }
       END { printf "      truncated_total=%d pages_total=%d max_pages_one_doorbell=%d (PT_SWEEP_BUDGET=300000 entries ≈ 585 pages/VAS)\n", t+0, p+0, mx+0 }' "$Q" 2>/dev/null
  echo "    --- sweep TRIGGERS actually seen (NeverSwept / Truncated / Dirty):"
  grep -oE 'reasons=\{[^}]*\}' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | head -8 | sed 's/^/      /'

  # =========================================================================================
  # ★★★★★ ARM 2.1 — LEAF PRESENT OR ABSENT. THE ARM THAT CAN KILL THE STORY.
  # =========================================================================================
  echo "--- ★★★★★ ARM 2.1: WHAT THE GUEST'S OWN TABLES DESCRIBE, vs WHERE HARDWARE FAULTED"
  echo "    --- the fault addresses hardware reported this boot:"
  local FAULTS
  FAULTS=$(grep -oE 'faulted @ 0x[0-9a-f_]+' "$D" 2>/dev/null | sed 's/faulted @ //' | sort -u)
  echo "      [${FAULTS:-⊘ NONE — and an EMPTY host dmesg is only 'no fault' if the watermark says so}]"
  grep -E 'HOST_DMESG_(LINES|XID)=|watermark' "$P" 2>/dev/null | sed 's/^/        /' | head -4
  echo "    --- the last GUEST-DESCRIBES row per address space (coalesced reachable VA runs):"
  grep -o 'GUEST-DESCRIBES.*' "$Q" 2>/dev/null | tail -3 | cut -c1-1200 | sed 's/^/      /'
  echo "    --- ★★★★★ THE JOIN, per fault address (⊘ done HERE; the device knows no address):"
  if [ -z "$FAULTS" ]; then
    echo "      ⊘ NOT ASKED — no fault address was reported. That is an absence of the QUESTION,"
    echo "        not an answer of LEAF-ABSENT, and it must never be read as one."
  else
    for fa in $FAULTS; do
      # The runs print as `0x<va>+0x<len>`; a containment test needs arithmetic, so awk does it.
      python3 - "$Q" "$fa" <<'PY' 2>/dev/null || echo "      ⊘ JOIN SCRIPT FAILED for $fa — no verdict, and this is NOT LEAF-ABSENT"
import re, sys
log, fa = sys.argv[1], sys.argv[2]
addr = int(fa.replace('_', ''), 16)
runs = []
for line in open(log, errors='replace'):
    if 'GUEST-DESCRIBES' not in line:
        continue
    for va, ln in re.findall(r'0x([0-9a-f]+)\+0x([0-9a-f]+)', line):
        runs.append((int(va, 16), int(ln, 16)))
hit = [(v, l) for v, l in runs if v <= addr < v + l]
capped = 'CAPPED' in open(log, errors='replace').read()[:0] or False
print(f"      {fa}: runs_printed={len(set(runs))} → ", end='')
if hit:
    v, l = hit[0]
    print(f"★★★ LEAF-PRESENT in a run 0x{v:x}+0x{l:x} ⇒ THE GUEST DESCRIBES IT and our "
          f"mirror missed it ⇒ the sweep is aimed right")
elif not runs:
    print("⊘ NO RUNS PRINTED AT ALL — the sweep published no picture, so this is NOT "
          "'LEAF-ABSENT'; it is 'NOT MEASURED'")
else:
    print("★★★ LEAF-ABSENT — no reachable run of any address space covers it ⇒ the guest "
          "never described it ⇒ MIRRORING CANNOT BIND IT and the sweep story is dead for "
          "this address. ⚠ Valid ONLY if no run list was CAPPED; check the ⚠⚠ CAPPED marker "
          "on the GUEST-DESCRIBES rows above.")
PY
    done
  fi
  echo "    --- ⚠ CAPPED markers on the range lists (a capped list makes ABSENT unreadable):"
  echo "        ranges: $(grep -c 'CAPPED at 48' "$Q" 2>/dev/null) row(s) | refused_vas: $(grep -c 'CAPPED at 24' "$Q" 2>/dev/null) row(s)"
  # ⊘⊘ THE SECOND HALF OF ARM 2.1, and without it the first half is only half an answer.
  #    LEAF-PRESENT says the guest describes the address. It does NOT say why nothing backs
  #    it. `refused_vas` is the table's own answer, and joining it against the fault address
  #    is what turns "the sweep is aimed right" into "and here is what stopped it".
  echo "    --- ★★★★★ WAS THE FAULTING ADDRESS ONE THE TABLE REFUSED?"
  echo "        refusal kinds, summed over the boot:"
  grep -oE 'by_kind=\{[^}]*\}' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | head -6 | sed 's/^/          /'
  for fa in $FAULTS; do
    fah=$(echo "$fa" | tr -d '_')
    echo "        $fa in a refused_vas list? $(grep -c "refused_vas=\[[^]]*${fah#0x}" "$Q" 2>/dev/null) row(s)"
  done
  echo "        distinct refused VAs seen anywhere this boot:"
  grep -oE 'refused_vas=\[[^]]*\]' "$Q" 2>/dev/null | tr ',[]' '\n\n\n' | grep -oE '0x[0-9a-f]+' \
    | sort -u | tr '\n' ' ' | fold -w 160 | sed 's/^/          /'

  # =========================================================================================
  # ★★★★★ THIS RUNG'S HEADLINE #1 — WHAT ACTUALLY DIFFERS BETWEEN THE TWO SHAPES
  # =========================================================================================
  echo "--- ★★★★★ THE STRADDLE CENSUS (w276 could print only 'StraddlesLiveBinding: 255')"
  echo "    --- every distinct straddles= summary line, with how often it occurred:"
  grep -oE 'straddles=[0-9]+ contradicting=[0-9]+ sigs=\{[^}]*\}' "$Q" 2>/dev/null \
    | sort | uniq -c | sort -rn | head -10 | sed 's/^/      /'
  echo "    --- ★★★★★ AGREEMENT, the field to read FIRST (SameMemory = one fact at two"
  echo "        granularities, nobody is wrong; Contradicts = two sources disagree ⇒ a BUG):"
  echo "        signatures carrying /SameMemory/  = $(grep -oE '[A-Za-z]+/SameMemory/[^,}]*' "$Q" 2>/dev/null | sort -u | wc -l) distinct"
  echo "        signatures carrying /Contradicts/ = $(grep -oE '[A-Za-z]+/Contradicts/[^,}]*' "$Q" 2>/dev/null | sort -u | wc -l) distinct"
  echo "        every distinct signature seen anywhere this boot:"
  grep -oE 'sigs=\{[^}]*\}' "$Q" 2>/dev/null | tr ',{}' '\n\n\n' | grep -oE '[A-Za-z]+/[A-Za-z]+/lvl[0-9]+/leaf0x[0-9a-f]+/live0x[0-9a-f]+/pub[01]=[0-9]+' \
    | sed -E 's/=[0-9]+$//' | sort -u | sed 's/^/          /'
  echo "        ⊘ contradicting=0 across every row ⇒ NO straddle indicts either source; the"
  echo "          refusal is the table declining to hold one range at two granularities, and"
  echo "          that is NOT the same finding as 'two populate sources disagree'."
  echo "    --- the FIRST straddle, whole (both shapes verbatim — reconstructible without this boot):"
  grep -oE 'first_straddle=\[[^]]*\]' "$Q" 2>/dev/null | sort -u | head -6 | sed 's/^/      /'
  echo "    --- ⚠ signature-cap markers (a capped signature list makes 'that shape did not occur' unreadable):"
  echo "        $(grep -c 'CAPPED at 12 of' "$Q" 2>/dev/null) row(s)"

  # =========================================================================================
  # ★★★★★ THIS RUNG'S HEADLINE #2 — THE THIRD OUTCOME
  # =========================================================================================
  echo "--- ★★★★★ SHAPE COLLISIONS: a leaf that reached NEITHER bound NOR refusals"
  echo "    --- every distinct shape_collisions= summary, with how often it occurred:"
  grep -oE 'shape_collisions=[0-9]+ dup_leaves=[0-9]+( by=\{[^}]*\})?' "$Q" 2>/dev/null \
    | sort | uniq -c | sort -rn | head -10 | sed 's/^/      /'
  echo "    --- the FIRST collision, whole (both leaves + the page each came out of):"
  grep -oE 'first_collision=\[[^]]*\]' "$Q" 2>/dev/null | sort -u | head -6 | sed 's/^/      /'
  echo "    ⊘ shape_collisions=0 on EVERY row does NOT mean the counter cannot fire — it is"
  echo "      pinned by tests/tests/straddle_shapes.rs, which builds the dual-PDE shape that"
  echo "      produces one and asserts it is recorded. A zero here is a fact about THIS GUEST."
  echo "    ⇒ if it is 0, the faulting VA was NOT lost here, and TABLE-DESCRIBES below is the"
  echo "      only remaining way to tell 'already bound' from 'never desired'."

  # =========================================================================================
  # ★★★★★ THIS RUNG'S HEADLINE #3 — IS THE FAULTING ADDRESS **BOUND**?
  # =========================================================================================
  echo "--- ★★★★★ TABLE-DESCRIBES: what OUR OWN address table holds, joined against the fault"
  echo "    --- the last TABLE-DESCRIBES row (our table's coalesced runs, per address space):"
  grep -o 'TABLE-DESCRIBES.*' "$Q" 2>/dev/null | tail -1 | cut -c1-1600 | sed 's/^/      /'
  if [ -z "$FAULTS" ]; then
    echo "      ⊘ NOT ASKED — no fault address was reported. An absence of the QUESTION."
  else
    for fa in $FAULTS; do
      python3 - "$Q" "$fa" <<'PY' 2>/dev/null || echo "      ⊘ TABLE JOIN SCRIPT FAILED for $fa — no verdict, and this is NOT 'unbound'"
import re, sys
log, fa = sys.argv[1], sys.argv[2]
addr = int(fa.replace('_', ''), 16)
rows = [l for l in open(log, errors='replace') if 'TABLE-DESCRIBES' in l]
runs, capped = set(), False
for line in rows:
    tail = line.split('TABLE-DESCRIBES', 1)[1]
    if 'CAPPED' in tail:
        capped = True
    for va, ln in re.findall(r'0x([0-9a-f]+)\+0x([0-9a-f]+)', tail):
        runs.add((int(va, 16), int(ln, 16)))
hit = [(v, l) for v, l in runs if v <= addr < v + l]
print(f"      {fa}: table_rows_printed={len(rows)} table_runs={len(runs)} → ", end='')
if not rows:
    print("⊘ NO TABLE PICTURE PRINTED — this is NOT 'unbound', it is NOT MEASURED")
elif hit:
    v, l = hit[0]
    print(f"★★★★★ **BOUND** — our table holds 0x{v:x}+0x{l:x} covering it ⇒ the mapping is "
          f"NOT missing, and 'our mirror missed it' is DEAD for this address")
elif capped:
    print("⊘ NOT IN THE PRINTED RUNS, **but a run list was CAPPED** ⇒ NOT MEASURED, and it "
          "must not be read as UNBOUND")
else:
    print("★★★ **UNBOUND** — no run of any address space covers it, and no list was capped "
          "⇒ the table genuinely does not hold it, and (with shape_collisions=0 and no "
          "refusal) it was never even DESIRED. That is the third state, still unnamed.")
PY
    done
  fi

  # =========================================================================================
  # ★★★★★ ARM 2.2 — THE LIVE WRITE CENSUS ON THE COMPLETION PAGE
  # =========================================================================================
  echo "--- ★★★★★ ARM 2.2: SEMA-WRITE-CENSUS (frozen-after-nothing vs frozen-after-corruption)"
  grep -o 'SEMA-WRITE-CENSUS.*' "$Q" 2>/dev/null | sed 's/^/      /' | tail -3
  echo "    --- every SEMA-WRITE transition, in order (this is the artefact; no cap):"
  grep -o 'SEMA-WRITE t=.*' "$Q" 2>/dev/null | cut -c1-220 | sed 's/^/      /' | head -60
  echo "    --- ★★★ BACKWARDS transitions (the M5.38 second-writer signature) = $(grep -c 'BACKWARDS' "$Q" 2>/dev/null)"
  echo "    --- carried: SEMA-PAGE-SLOT=$(grep -c 'SEMA-PAGE-SLOT' "$Q" 2>/dev/null) SEMA-PAGE-ZERO=$(grep -c 'SEMA-PAGE-ZERO' "$Q" 2>/dev/null) COMPLETION-WATCH →OBSERVED=$(grep -c 'COMPLETION-WATCH.*→ OBSERVED' "$Q" 2>/dev/null) →NOT-OBSERVED=$(grep -c 'COMPLETION-WATCH.*→ NOT-OBSERVED' "$Q" 2>/dev/null)"
  echo "      ⊘ ANCHORED ON THE ARROW. \`grep -c 'COMPLETION-WATCH.*OBSERVED'\` reads 97 on a boot"
  echo "        with EIGHT completions: it also matches two long prose lines carrying both words"
  echo "        and no verdict. An unanchored count of a verdict is not a count of that verdict."

  # =========================================================================================
  # ★★★★★ THE CE-PT-WRITE PRODUCER'S LIVENESS — the coordinator's question, as a count
  # =========================================================================================
  echo "--- ★★★★★ IS THE CE-PT-WRITE PRODUCER ON THE PATH AT ALL?"
  echo "    FWD-RING lines = $(grep -c 'FWD-RING' "$Q" 2>/dev/null)  (w275_pin: 0)"
  echo "    ⊘ FWD-RING is the ONLY caller of parse_pushbuffer, which is the only producer of"
  echo "      PtWrite — the port's analogue of the C's nvkvm_m2_ce_fb_write_hook. A ZERO here"
  echo "      means that producer NEVER FIRED this boot, so Vas::pt_pages was fed by the"
  echo "      CPU/BAR2 witness alone. ⇒ the C's second mechanism has no live input here."
  echo "    engines that reached the forwarding fall-through:"
  grep -oE 'RING-PROJ .*engine=[A-Za-z]+' "$Q" 2>/dev/null | grep -oE 'engine=[A-Za-z]+' | sort | uniq -c | sed 's/^/      /'
  echo "    --- CARRIED: PT-DECODE (the DIRECT decode of dirtied pages — the C's cpt_sync half):"
  grep -oE 'PT-DECODE drained=[0-9]+ latched=[0-9]+[^|]*bound=[0-9]+ unchanged=[0-9]+' "$Q" 2>/dev/null \
    | sort | uniq -c | sort -rn | head -6 | sed 's/^/      /'

  # =========================================================================================
  # ★★★★★ THE Xid, BY IDENTITY, NEVER BY COUNT — w265's lesson.
  # =========================================================================================
  echo "--- ★★★★★ Xid IDENTITY:"
  grep -o 'Xid.*' "$D" 2>/dev/null | cut -c1-200 | sort -u | sed 's/^/      /'
  echo "      ENGINE/CLIENT pairs = [$(grep -oE 'ENGINE [A-Z0-9_]+ HUBCLIENT_[A-Z0-9_]+' "$D" 2>/dev/null | sort -u | tr '\n' ' ')]"
  echo "      ACCESS types        = [$(grep -oE 'ACCESS_TYPE_[A-Z_]+' "$D" 2>/dev/null | sort -u | tr '\n' ' ')]"
  echo "      FAULT types         = [$(grep -oE 'FAULT_[A-Z]+' "$D" 2>/dev/null | sort -u | tr '\n' ' ')]"
  echo "      ⚠ engine and channel are ONE measurement reported twice (kchannelGetDebugTag"
  echo "        returns (runlistId<<24)|ChID) — never tallied as two."

  # =========================================================================================
  # ★★★★★ CUP2_RC — ANCHORED. The anchor is the whole point.
  # =========================================================================================
  # ⊘⊘ `grep -o 'CUP2_RC=[0-9]*'` ALSO matches `GCC_CUP2_RC=0`, the guest COMPILER's status,
  #    which is 0 on every healthy boot. `[measured, w271_off]` the unanchored form reported
  #    CUP2_RC=0 — THE CAMPAIGN'S HEADLINE SUCCESS VALUE — on an arm that was hanging.
  local rc
  rc=$(grep -oE '(^|[^A-Z_])CUP2_RC=[0-9]+' "$P" 2>/dev/null | grep -oE 'CUP2_RC=[0-9]+' | tail -1)
  echo "--- ★★★★★ CUP2_RC = [${rc:-⊘ NO cup2 EXIT LINE — THE MEASUREMENT DID NOT HAPPEN. ⊘ This is NOT 0 and must never be read as one}]"
  echo "    (⊘ disambiguated from GCC_CUP2_RC, the compiler's status: $(grep -c 'GCC_CUP2_RC' "$P" 2>/dev/null) such line(s) present)"
  echo "    unanchored, for contrast: [$(grep -oh 'CUP2_RC=[0-9]*' "$P" 2>/dev/null | tr '\n' ' ')]"
  echo "--- cup2's own last prints:"
  grep -E '^ok |^CUP2_RC|totalMem' "$P" 2>/dev/null | tail -12 | sed 's/^/      /'
  echo "--- the probe's headlines:"
  grep -E 'POLLED ADDRESS|SLOT-JOIN|VALUE AT IT|reached-cuCtxCreate|SPINPROBE_RC=' "$P" 2>/dev/null \
    | sed 's/^/      /' | head -30
}

#     tag        FB_JOIN GUEST_RING GUEST_PUSHBUF PT_WITNESS_EXEC GUEST_SEMA GR_ROUTE     GUEST_OPERAND PT_SWEEP
# ★ The ARMED arm runs FIRST, deliberately: if the session is cut short, the rung's own
#   measurement exists and the control is the one that is missing — which is recoverable, and
#   the other order is not. ⊘ w271_pin/w274_pin/w275_pin are near-controls at older builds.
# ⊘ Selectable so a re-run that only needs to re-measure the ARMED arm — e.g. after an
#   INSTRUMENT fix, which is not a device change — does not spend a second boot re-confirming a
#   control whose device is byte-identical. ⚠ A run that skips the control must SAY it did;
#   the line below is what says it.
ARMS=${W277_ARMS:-"on off"}
echo "=== ARMS TO RUN: [$ARMS] $([ "$ARMS" = "on off" ] || echo '⚠ NOT THE FULL PAIR — this run has no control of its own; name the boot it borrows one from')"
for a in $ARMS; do
  boot ${PFX}_${a} shared ring pin on pin passthrough pin "$a"
done

echo "=== ARTEFACT SIZES ==="
ls -l /workspace/bench/run_${PFX}_* 2>/dev/null
finish 0
