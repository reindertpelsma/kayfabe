#!/usr/bin/env bash
# w271 — THE EXTENT KEY. Two boots, ONE variable: `KAYFABE_GUEST_OPERAND`.
#
#   arm   FB_JOIN  GUEST_RING  GUEST_PUSHBUF  PT_WITNESS_EXEC  GUEST_SEMA  GR_ROUTE      GUEST_OPERAND
#   off   shared   ring        pin            on               pin         passthrough   (unset)
#   pin   shared   ring        pin            on               pin         passthrough   pin
#
# ⊘ THE ARMING IS w270'S, UNCHANGED — deliberately. The variable under test this rung is NOT
# an env flag at all: it is the pin primitive's IDENTITY, which is now the `(base, extent)`
# pair rather than the base. So `w271_off` must reproduce `w270_off` line for line, and
# `w271_pin` differs from `w270_pin` ONLY in that a growing run is now described.
#
# ★★★★★ WHY: `w270_pin` placed 32 KiB at `0x2_04420000`, the submission retired, the polled
# slot went 1 → 2 for the first time in the campaign — and the fault MOVED to `0x2_04428000`,
# exactly one extent on. The log carried its own cause:
#     operand run 1/1 va=0x204420000 … len=32768 → PINNED         memory=0xcafe0055
#     operand run 1/1 va=0x204420000 … len=65536 → ALREADY PINNED memory=0xcafe0055
# `pin_guest_ram`'s idempotence key was the VA, so a 64 KiB run at a base described for
# 32 KiB replayed the short descriptor and reported `placed_as_asked=true`. ⇒ A GREEN VERDICT
# ON A PARTIAL MAPPING. Full argument and every prediction:
# `docs/design/w271_the_extent_key_prereg.md` (committed BEFORE any of this rung's code).
#
# ⊘⊘ BOTH ARMS CARRY `GR_ROUTE=passthrough`, and that is deliberate: the wall under test only
# EXISTS on that arm — `refuse` never reaches chan 8's second submission at all. ⇒ `w271_off`
# is a REPLICATION of `w270_off` and is graded as one. The route is still NOT defaulted.
#
# ⚠⚠ `0x2_04420000` and `0x2_04428000` appear in this script ONLY in the grader. They must
# appear ZERO times in the device binary — a pin that RECALLS an address instead of DECODING
# one would make every row below read as evidence for a mechanism that is a constant.
#
# ★ START marker and EXIT line so "file exists but has no terminator" is detectable at all
#   (143 = the JOB was SIGTERMed; 124 = the LAUNCHER's ssh expired while the job ran on —
#   opposite meanings arriving as the same word).
PFX=${W271_TAG_PREFIX:-w271}
OUT=/workspace/${PFX}_run.log
exec >"$OUT" 2>&1
finish() { echo "=== W271 EXIT rc=$1 at $(date -Is) ==="; exit "$1"; }
echo "=== W271 START $(date -Is) pid=$$ ==="

export PATH=/root/.cargo/bin:$PATH
REPO=${KAYFABE_REPO:-/workspace/kayfabe_w271}
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
# ⚠ From the SAME invocation, per CLAUDE.md: disk is the failure that reads as a code error.
echo "=== BUILD ENOSPC/LLVM = $(grep -c 'No space left on device\|LLVM ERROR' "$OUT" 2>/dev/null) | df: $(df -h /workspace | tail -1) ==="

# ★★★ THE STAMP GATE — the ANCHORED form (exactly 40 hex), from `w265_run.sh`.
# ⊘⊘ DO NOT copy `w263`/`w264`'s unbounded `[0-9a-f]*`: it swallows the next `.rodata` literal
#    when that starts with a hex digit, and its message is INDISTINGUISHABLE from the real
#    staleness it guards against.
STAMP=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 \
        | grep -oE 'kayfabe-rev:[0-9a-f]{40}(-dirty)?' | sort -u)
echo "=== STAMP: [$STAMP] WANT: [kayfabe-rev:$HEAD] ==="
NSTAMP=$(printf '%s\n' "$STAMP" | grep -c .)
if [ "$NSTAMP" != "1" ]; then
  echo "=== ★★★ THE EXTRACTION IS THE PROBLEM, NOT THE BINARY: found $NSTAMP stamps, wanted 1."
  echo "===     ⊘ Do NOT read this as a stale build until the extractor is exonerated. ==="
  finish 95
fi
if [ "$STAMP" != "kayfabe-rev:$HEAD" ]; then
  echo "=== ★★★ STAMP MISMATCH — REFUSING TO BOOT. ==="
  finish 93
fi
echo "=== STAMP GATE: PASS ==="
echo "kayfabe-rev:$HEAD" > /workspace/bench/BUILD_REV.txt

# ★ CONTENT CHECK — asserted, not printed. A stamp says which REVISION; these say which CODE.
#   ⊘ The FIRST SIX are THIS rung's own strings. A zero for any of them means the boot would
#     measure a binary in which the source under test does not exist, and every `OPERAND-*`
#     row would be absent BY CONSTRUCTION — indistinguishable, in the log, from a submission
#     that named no operand. That is the zero-byte-artefact trap wearing a different hat.
echo "=== CONTENT CHECK ==="
CC_RC=0
# ⊘ THE FIRST FIVE ARE w271'S OWN STRINGS — the three verdict words that must never share,
#   and the `requested=`/`described=` pair. A zero for any of them means the boot would
#   measure a binary in which the extent key does not exist, and every row would print the
#   OLD single green word. That is indistinguishable, in the log, from "nothing ever grew".
for s in "GREW (partial hit" "ALREADY PINNED (idempotent replay; fully covered)" \
         "requested=" "described=" "GuestRamPinTooShort" "GuestRamPinOverlaps" \
         "OPERAND-PIN token=" "OPERAND-SOURCE-CE" "OPERAND-TABLE:" "NO OPERAND PAGE TO PIN" \
         "KAYFABE_GUEST_OPERAND" "GUEST-OPERAND arm=" "OPERAND RUN(S) PLACED" \
         "SEMA-SOURCE-CE" "SEMA-SOURCES:" "SEMA-PIN" "SEMA-TABLE:" "KAYFABE_GUEST_SEMA" \
         "GUEST-SEMA arm=" "KAYFABE_GR_ROUTE" "GR-ROUTE arm=" "PB-PIN" "EXEC-WITNESS ARMED" \
         "COMPLETION-WATCH" "SEMA-PAGE seq=" "SEMA-PAGE-SLOT" "GR-CURSOR token=" \
         "DOORBELL-XLATE" "VAS-BIND-CENSUS" "PT-DECODE"; do
  n=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 | grep -c -- "$s")
  printf '  %-38s = %s\n' "$s" "$n"
  [ "$n" -gt 0 ] || { echo "  ★★★ ZERO — the code under test is NOT in this binary"; CC_RC=1; }
done
# ⊘⊘ AND THE NEGATIVE CONTENT CHECK, which is this rung's own `cap2b` guard: the faulting
#    address must NOT be a literal in the device. If it is, the pin is recalling an address
#    instead of decoding one, and every row below would read as evidence for a mechanism that
#    is actually a remembered constant.
NCAP=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 \
       | grep -c '0x2_04420000\|0x204420000\|0x2_04428000\|0x204428000')
echo "  ★★★ cap2b GUARD: literal 0x2_0442{0,8}000 in the binary = $NCAP (MUST be 0)"
[ "$NCAP" -eq 0 ] || { echo "  ★★★ THE ADDRESS IS A LITERAL — the pin recalls, it does not decode"; CC_RC=1; }
# ⊘⊘ AND THE SECOND NEGATIVE CHECK, which is THIS rung's own: the OLD verdict string must be
#    GONE. `ALREADY PINNED (idempotent replay)` with no `; fully covered` was the word that
#    covered BOTH "it was already complete" and "we only did part of it". If it is still in
#    the binary, some call site was missed and its rows cannot be told apart.
NOLD=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 \
       | grep -c 'ALREADY PINNED (idempotent replay)$')
echo "  ★★★ FALSE-GREEN GUARD: the OLD un-qualified verdict word = $NOLD (MUST be 0)"
[ "$NOLD" -eq 0 ] || { echo "  ★★★ A CALL SITE STILL SHARES ONE WORD ACROSS FULL AND PARTIAL"; CC_RC=1; }
[ $CC_RC -eq 0 ] || finish 94

export KAYFABE_ISOLATES=real
export KAYFABE_CE_EXECUTOR=host
export NVKVM_RAM_BACKEND=memfd
export KAYFABE_GUEST_RAM=memfd
# ★ The w269 hook: it launches cup2 DETACHED, waits for `totalMem=` (cup2's own last print
#   before `cuCtxCreate`), then runs the ptrace spin probe TWICE. It is the only instrument
#   that can answer "did 0x2_0440ff70 reach 2" and it also produces CUP2_RC.
#   ⊘ `gcup2_gdb.sh` cannot: it is a SIGSEGV instrument and prints nothing on a hang, and gdb
#     is absent from the guest and uninstallable (w269 §0.3).
export POST_CAPTURE_HOOK=$REPO/scripts/bench/cup2_hook_gdbspin.sh
export GQ_TIMEOUT=420
export BOOT_TIMEOUT=180
for f in scripts/bench/guest_spinprobe.c scripts/bench/cup2_hook_gdbspin.sh; do
  [ -f "$f" ] || { echo "=== ★★★ MISSING $f — the probe cannot run ==="; finish 96; }
done

# ⊘ EVERY per-arm variable is unset first and set only if this arm names it, so an arm can
#   never inherit the previous arm's arming.
boot() {
  local tag=$1 fbj=$2 ring=$3 pb=$4 wit=$5 sema=$6 route=$7 op=$8
  unset KAYFABE_FB_JOIN KAYFABE_GUEST_RING KAYFABE_GUEST_PUSHBUF KAYFABE_PT_WITNESS_EXEC \
        KAYFABE_GUEST_SEMA KAYFABE_GR_ROUTE KAYFABE_GUEST_OPERAND
  [ "$fbj"   = "-" ] || export KAYFABE_FB_JOIN=$fbj
  [ "$ring"  = "-" ] || export KAYFABE_GUEST_RING=$ring
  [ "$pb"    = "-" ] || export KAYFABE_GUEST_PUSHBUF=$pb
  [ "$wit"   = "-" ] || export KAYFABE_PT_WITNESS_EXEC=$wit
  [ "$sema"  = "-" ] || export KAYFABE_GUEST_SEMA=$sema
  [ "$route" = "-" ] || export KAYFABE_GR_ROUTE=$route
  [ "$op"    = "-" ] || export KAYFABE_GUEST_OPERAND=$op
  echo "=== BOOT $tag START $(date -Is) ==="
  echo "    KAYFABE_FB_JOIN=[${KAYFABE_FB_JOIN:-unset}]" \
       "KAYFABE_GUEST_RING=[${KAYFABE_GUEST_RING:-unset}]" \
       "KAYFABE_GUEST_PUSHBUF=[${KAYFABE_GUEST_PUSHBUF:-unset}]" \
       "KAYFABE_PT_WITNESS_EXEC=[${KAYFABE_PT_WITNESS_EXEC:-unset}]" \
       "KAYFABE_GUEST_SEMA=[${KAYFABE_GUEST_SEMA:-unset}]" \
       "KAYFABE_GR_ROUTE=[${KAYFABE_GR_ROUTE:-unset}]" \
       "KAYFABE_GUEST_OPERAND=[${KAYFABE_GUEST_OPERAND:-unset}]"
  timeout 1200 "$REPO/scripts/bench/boot_capture.sh" "$tag"
  echo "=== BOOT $tag RC=$? $(date -Is) ==="
  local Q=/workspace/bench/run_${tag}_qemu.log
  local P=/workspace/bench/run_${tag}_probe.log
  local D=/workspace/bench/run_${tag}_hostdmesg.log
  # ⊘ pgrep -x qemu-system-x86 (comm truncates at 15); NOT -f (it matches the asker).
  echo "--- post-boot liveness: pgrep -x qemu-system-x86 = [$(pgrep -x qemu-system-x86 | tr '\n' ' ')] ss2223 = [$(ss -tln 2>/dev/null | grep -c 2223)]"
  echo "--- ENOSPC_LLVM=$(grep -c 'No space left on device\|LLVM ERROR' "$Q" 2>/dev/null || echo '?')"

  # ★★ THE ARMING AS THE DEVICE SAW IT — six flags, read out of the boot's own log.
  echo "--- ARMING AS THE DEVICE SAW IT:"
  for pat in 'FB-JOIN arm=' 'GUEST-RING arm=' 'GUEST-PUSHBUF arm=' 'GUEST-SEMA arm=' \
             'GR-ROUTE arm=' 'GUEST-OPERAND arm='; do
    echo "    $(grep -m1 -o "kayfabe: ${pat}[a-z]*" "$Q" 2>/dev/null || echo "(no ${pat} line)")"
  done
  echo "    $(grep -m1 -o 'EXEC-WITNESS [A-Z]*[^|]*' "$Q" 2>/dev/null | cut -c1-120 || echo '(no EXEC-WITNESS line)')"

  # ★★★★★ THE ASSERTIONS, not prints. The rung is ONE variable; an arm that took the wrong
  #        one is not a data point, it is a duplicate of the other arm.
  local wantop='off'; [ "$op" = "pin" ] && wantop='pin'
  if grep -q "GUEST-OPERAND arm=$wantop" "$Q" 2>/dev/null; then
    echo "    ★ OPERAND-ARM ASSERTION: PASS (saw GUEST-OPERAND arm=$wantop)"
  else
    echo "    ★★★ OPERAND-ARM ASSERTION: FAIL — wanted GUEST-OPERAND arm=$wantop. ⊘ VOID."
  fi
  local wantroute='refuse'; [ "$route" = "passthrough" ] && wantroute='passthrough'
  grep -q "GR-ROUTE arm=$wantroute" "$Q" 2>/dev/null \
    && echo "    ★ ROUTE-ARM ASSERTION: PASS (GR-ROUTE arm=$wantroute)" \
    || echo "    ★★★ ROUTE-ARM ASSERTION: FAIL — wanted GR-ROUTE arm=$wantroute. ⊘ VOID."
  for pair in "GUEST-SEMA arm=$sema" "GUEST-PUSHBUF arm=$pb" "GUEST-RING arm=$ring"; do
    grep -q "$pair" "$Q" 2>/dev/null \
      && echo "    ★ CARRIED-ARM ASSERTION: PASS ($pair)" \
      || echo "    ★★★ CARRIED-ARM ASSERTION: FAIL — wanted '$pair'. ⊘ VOID."
  done
  local wantwit='DISARMED'; [ "$wit" = "on" ] && wantwit='ARMED'
  grep -q "EXEC-WITNESS $wantwit" "$Q" 2>/dev/null \
    && echo "    ★ WITNESS-ARM ASSERTION: PASS (EXEC-WITNESS $wantwit)" \
    || echo "    ★★★ WITNESS-ARM ASSERTION: FAIL — wanted EXEC-WITNESS $wantwit. ⊘ VOID."

  # ⊘ THE CONTROL'S OWN EXPECTED RESULT, asserted as an ABSENCE WITH A NUMBER.
  echo "    --- OPERAND-PIN line count = $(grep -c 'OPERAND-PIN' "$Q" 2>/dev/null) (control wants 0, rung wants >0)"

  # ★★★★★ THIS RUNG'S HEADLINE ROWS — the source, verbatim, per doorbell.
  echo "    --- ★★★ OPERAND-SOURCE-CE, every row (the guest's own declaration):"
  grep -o 'OPERAND-SOURCE-CE token=[^⊘]*' "$Q" 2>/dev/null | cut -c1-260 | sed 's/^/      /' | head -30
  echo "    --- ★★★ OPERAND-TABLE verdicts:"
  grep -o 'OPERAND-TABLE:.*' "$Q" 2>/dev/null | cut -c1-220 | sort | uniq -c | sed 's/^/      /' | head -20
  echo "    --- ★★★ OPERAND runs PLACED / REFUSED:"
  grep -oE 'operand run [0-9]+/[0-9]+ va=0x[0-9a-f]+ gpa=0x[0-9a-f]+ len=[0-9]+ → [^m]*' "$Q" 2>/dev/null \
    | cut -c1-200 | sed 's/^/      /' | head -20

  # ★★★★★ THIS RUNG'S OWN ROWS — THE EXTENT KEY, graded by IDENTITY and not by count.
  echo "    --- ★★★★★ w271 THE THREE VERDICT WORDS (they must NEVER share a row):"
  echo "        PINNED (fresh, whole)      = $(grep -c ' PINNED requested=' "$Q" 2>/dev/null)"
  echo "        ALREADY (fully covered)    = $(grep -c 'ALREADY PINNED (idempotent replay; fully covered)' "$Q" 2>/dev/null)"
  echo "        ★★★ GREW (partial hit)     = $(grep -c 'GREW (partial hit' "$Q" 2>/dev/null)   ← the rung fires iff this is > 0 on the pin arm"
  echo "    --- ★★★★★ w271 EVERY GREW ROW, VERBATIM (requested vs described, side by side):"
  grep -oE '(operand|sema|pb|) ?run [0-9]+/[0-9]+ va=0x[0-9a-f]+[^|]*GREW[^|]*' "$Q" 2>/dev/null \
    | cut -c1-300 | sed 's/^/      /' | head -20
  # ⚠ `run N/M` is a SUBSTRING of `pb run N/M`, `sema run N/M` and `operand run N/M`, so the
  #   three named sources are counted first and the RING is the remainder. A naive loop over
  #   the four patterns would have reported the ring's count as the total, on every arm.
  echo "    --- ★★★★★ w271 WHICH SOURCES GREW (the brief asked whether all four benefit):"
  cnt() { grep -cE "$1" "$Q" 2>/dev/null || echo 0; }
  ALLG=$(cnt ' run [0-9]+/[0-9]+ .*GREW'); ALLP=$(cnt ' run [0-9]+/[0-9]+ .* PINNED requested='); ALLA=$(cnt ' run [0-9]+/[0-9]+ .*fully covered')
  SUMG=0; SUMP=0; SUMA=0
  for src in 'pb run' 'sema run' 'operand run'; do
    g=$(cnt "$src [0-9]+/[0-9]+ .*GREW"); q=$(cnt "$src [0-9]+/[0-9]+ .* PINNED requested="); a=$(cnt "$src [0-9]+/[0-9]+ .*fully covered")
    printf '        %-14s GREW=%s  PINNED=%s  ALREADY=%s\n' "$src" "$g" "$q" "$a"
    SUMG=$((SUMG+g)); SUMP=$((SUMP+q)); SUMA=$((SUMA+a))
  done
  printf '        %-14s GREW=%s  PINNED=%s  ALREADY=%s   (⊘ DERIVED as total minus the three named)\n' \
    'ring run' "$((ALLG-SUMG))" "$((ALLP-SUMP))" "$((ALLA-SUMA))"
  # ⊘⊘ THE FALSE-GREEN ASSERTION. This is the property the whole rung is FOR: no row may
  #    ever report a replay while `described < requested`. It is asserted as a COUNT of the
  #    violating shape, because a reader cannot be asked to compare two numbers on 40 rows.
  echo "    --- ⊘⊘ FALSE-GREEN CHECK — rows claiming a replay with described < requested:"
  BADG=$(grep -oE 'requested=[0-9]+ described=[0-9]+' "$Q" 2>/dev/null \
         | awk -F'[= ]' '$2 > $4 {n++} END {print n+0}')
  echo "        rows with requested > described = $BADG (MUST be 0 — a nonzero is the w270 defect, alive)"
  echo "        rows printing BOTH numbers      = $(grep -c 'requested=[0-9]* described=' "$Q" 2>/dev/null) (a ZERO here means the binary is STALE, not that nothing grew)"
  # ⊘ The refusals, by name — a `GuestRamPinTooShort` that reaches a LOG rather than being
  #   consumed by the loop means the VMM half did not run.
  echo "    --- ⊘ pin refusals reaching the log (the loop should CONSUME TooShort/Overlaps):"
  grep -oE 'REFUSED (GuestRamPin[A-Za-z]+|[A-Za-z]+)[^|]{0,80}' "$Q" 2>/dev/null \
    | cut -c1-140 | sort | uniq -c | sed 's/^/      /' | head -10
  echo "    --- OPERAND verdict lines:"
  grep -o 'OPERAND RUN(S) PLACED[^|]*\|NO OPERAND PAGE TO PIN\|NOT ONE OPERAND PAGE RESOLVED' "$Q" 2>/dev/null \
    | cut -c1-160 | sort | uniq -c | sed 's/^/      /' | head

  # ★★★★★ **THE Xid, BY IDENTITY, NEVER BY COUNT** — w265's lesson. Engine, client, distinct
  #        addresses, access type. A count cannot see a substitution.
  echo "    --- ★★★★★ Xid IDENTITY (engine / client / distinct addrs / access type):"
  grep -o 'Xid.*' "$D" 2>/dev/null | cut -c1-200 | sort -u | sed 's/^/      /'
  echo "        distinct fault ADDRESSES = [$(grep -oE 'faulted @ 0x[0-9a-f_]+' "$D" 2>/dev/null | sort -u | tr '\n' ' ')]"
  echo "        ENGINE/CLIENT pairs      = [$(grep -oE 'ENGINE [A-Z0-9_]+ HUBCLIENT_[A-Z0-9_]+' "$D" 2>/dev/null | sort -u | tr '\n' ' ')]"
  echo "        ACCESS types             = [$(grep -oE 'ACCESS_TYPE_[A-Z_]+' "$D" 2>/dev/null | sort -u | tr '\n' ' ')]"
  echo "        ⊘ HOST_DMESG bytes=$(stat -c %s "$D" 2>/dev/null || echo '?') — a ZERO here is only 'no fault' if the probe log's watermark says so:"
  grep -E 'HOST_DMESG_(LINES|XID)=|watermark' "$P" 2>/dev/null | sed 's/^/          /' | head -4
  # ⊘⊘ THE JOIN THE WHOLE RUNG TURNS ON: did the address the GUEST declared match the one
  #    HARDWARE faulted on? ⚠ The literal lives HERE, in the grader, and nowhere in the device.
  echo "        ★★★ AGREEMENT: guest-declared operand names 0x204420000? $(grep -c 'OPERAND-SOURCE-CE.*0x204420000' "$Q" 2>/dev/null) | hardware faulted there? $(grep -c '0x2_04420000' "$D" 2>/dev/null)"
  # ⊘⊘ THE RUNG'S OWN FALSIFIER, BY IDENTITY. w270's wall was `0x2_04428000`. Three states,
  #    not two: still there (fix did not take) / gone (extent story closes) / a THIRD address
  #    (the fix is right and something else still truncates). A COUNT cannot tell them apart —
  #    `grep -c Xid` read `1` on BOTH of w270's arms while the identity moved +0x8000.
  echo "        ★★★★★ w271 FALSIFIER: w270 wall 0x2_04428000 present? $(grep -c '0x2_04428000' "$D" 2>/dev/null) | w270_off wall 0x2_04420000 present? $(grep -c '0x2_04420000' "$D" 2>/dev/null)"
  echo "                     ⇒ ALL distinct fault addrs this arm = [$(grep -oE 'faulted @ 0x[0-9a-f_]+' "$D" 2>/dev/null | sort -u | tr '\n' ' ')]"

  # ⊘ The carried guards from w268 — if these move, this rung's comparison changes meaning.
  echo "    --- CARRIED: COMPLETION-WATCH OBSERVED = $(grep -c 'COMPLETION-WATCH.*OBSERVED' "$Q" 2>/dev/null) | SEMA-PAGE-SLOT = $(grep -c 'SEMA-PAGE-SLOT' "$Q" 2>/dev/null) | PB-PIN PINNED = $(grep -c 'pb run.*PINNED' "$Q" 2>/dev/null)"
  echo "    --- CARRIED: PT-DECODE bound (1st) = $(grep -m1 -o 'bound=[0-9]*' "$Q" 2>/dev/null) | refusals = $(grep -m1 -o 'refusals=[0-9]*' "$Q" 2>/dev/null)"
  echo "    --- CARRIED: GR CURSOR PAIRS SEEN (distinct):"
  grep -o 'GR-CURSOR .*fbuserd@0x[0-9a-f]* GET=[0-9]* PUT=[0-9]*' "$Q" 2>/dev/null \
    | sed -E 's/.*(proc=[0-9]+ chan=[0-9]+).*(GET=[0-9]+ PUT=[0-9]+)/      \1 \2/' | sort -u | head -20
  echo "    --- CARRIED: DOORBELL witness XLATE=$(grep -c 'DOORBELL-XLATE' "$Q" 2>/dev/null) VERB=$(grep -c 'DOORBELL-VERB' "$Q" 2>/dev/null) STORE-WROTE=$(grep -c 'DOORBELL-STORE.*WROTE' "$Q" 2>/dev/null) NOT-REACHED=$(grep -c 'DOORBELL-STORE.*NOT REACHED' "$Q" 2>/dev/null)"

  # ★★★★★ THE PROBE — the only instrument that can say whether the polled slot MOVED.
  echo "    --- ★★★★★ THE PROBE'S OWN HEADLINES (does 0x20440ff70 reach 2?):"
  grep -E 'POLLED ADDRESS|SLOT-JOIN|N items =|KIND=|SNAPSHOT at|NO SNAPSHOT|steps actually taken|reached-cuCtxCreate|CUP2_RC=|SPINPROBE_RC=|VALUE AT IT|limit  =|cached =' \
    "$P" 2>/dev/null | sed 's/^/      /' | head -70
  # ⚠ The line above deliberately shows `GCC_CUP2_RC` too, unfiltered — so a reader can SEE
  #   the two rows that a single unanchored pattern would have collapsed into one.
  # ⊘⊘ **ANCHORED, AND THE ANCHOR IS THE WHOLE POINT.** `grep -o 'CUP2_RC=[0-9]*'` ALSO
  #    matches `GCC_CUP2_RC=0` — the guest COMPILER's exit status, which is `0` on every
  #    healthy boot. `[measured 2026-08-12, w271_off, caught before it reached a RESULT]` the
  #    unanchored form reported `CUP2_RC=0` — THE CAMPAIGN'S HEADLINE SUCCESS VALUE — on an
  #    arm that was hanging exactly as `w269b_pass` did, because cup2's own line had not been
  #    written yet. ⇒ On a boot where the measurement did NOT happen, the naive grader reports
  #    SUCCESS. Same class as the zero-byte artefact: an absent measurement reading as a
  #    favourable one, in the single most consequential direction this campaign has.
  #    ⚠ `tail -1` does not save it: it is only right while the real line happens to come last.
  local rc
  rc=$(grep -oE '(^|[^A-Z_])CUP2_RC=[0-9]+' "$P" 2>/dev/null | grep -oE 'CUP2_RC=[0-9]+' | tail -1)
  echo "    --- ★★★★★ CUP2_RC = [${rc:-⊘ NO cup2 EXIT LINE — THE MEASUREMENT DID NOT HAPPEN. ⊘ This is NOT 0 and must never be read as one}]"
  echo "        (⊘ disambiguated from GCC_CUP2_RC, the compiler's status: $(grep -c 'GCC_CUP2_RC' "$P" 2>/dev/null) such line(s) present)"
  echo "    --- cup2's own last prints:"
  grep -E '^ok |^CUP2_RC|totalMem' "$P" 2>/dev/null | tail -12 | sed 's/^/      /'
}

#     tag          FB_JOIN GUEST_RING GUEST_PUSHBUF PT_WITNESS_EXEC GUEST_SEMA GR_ROUTE     GUEST_OPERAND
boot ${PFX}_off   shared  ring       pin           on              pin        passthrough  -
boot ${PFX}_pin   shared  ring       pin           on              pin        passthrough  pin

echo "=== ARTEFACT SIZES ==="
ls -l /workspace/bench/run_${PFX}_* 2>/dev/null
finish 0
