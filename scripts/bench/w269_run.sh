#!/usr/bin/env bash
# w269 — READ THE POLLED ADDRESS. Two boots, ONE variable: `KAYFABE_GR_ROUTE`.
#
#   arm      FB_JOIN  GUEST_RING  GUEST_PUSHBUF  PT_WITNESS_EXEC  GUEST_SEMA  GR_ROUTE
#   refuse   shared   ring        pin            on               pin         (unset)
#   pass     shared   ring        pin            on               pin         passthrough
#
# ★★★★★ WHY: `w268` closed the completion plane — all eight GR slots written, payload 1,
# distinct GPU timestamps — and `CUP2_RC` did not move. The explanation the campaign ran on
# ("the guest waits for a semaphore nobody writes") is RETIRED, and what it waits on now is
# unmeasured. This rung reads the polled ADDRESS out of the spinning process with `ptrace`.
# Full argument, the disassembly it rests on, and every prediction:
# `docs/design/w269_the_spin_address_prereg.md` (committed BEFORE the probe existed).
#
# ⊘⊘ THIS RUNG BUILDS NOTHING AND CHANGES NO RUST. The binary under test is the one `w268`
# measured. ⇒ the gate below is not a stamp CHECK, it is a stamp REQUIREMENT: the binary must
# be exactly `70463ae…`, and the run REFUSES otherwise. A rung that measured a different
# binary than its predecessor could not be compared to it, which is the whole point.
# ★ Corollary: this rung is structurally immune to the "bench silently served a binary built
#   from 862c7c2" trap, because it never invokes a compiler.
# ★ TAG PREFIX so a SECOND pass (a deeper decode of the same question) does not overwrite the
#   first pass's artefacts. ⊘ Overwriting them would destroy the only evidence that the first
#   decode stopped where it said it did.
PFX=${W269_TAG_PREFIX:-w269}
OUT=/workspace/${PFX}_run.log
exec >"$OUT" 2>&1
finish() { echo "=== W269 EXIT rc=$1 at $(date -Is) ==="; exit "$1"; }
echo "=== W269 START $(date -Is) pid=$$ ==="

REPO=${KAYFABE_REPO:-/workspace/kayfabe_w269}
# ⊘ Pass 1 measured the w268 binary with NO rebuild (`WANT_BIN_REV` pinned to `70463ae`).
# Pass 2 carries the owner's DOORBELL-STORE witness, which is a Rust change, so it MUST build —
# and the requirement then becomes "the binary is THIS checkout", which is the ordinary stamp
# gate. ⚠ Set `W269_REBUILD=1` deliberately; the default stays the no-build pin, because a
# rung that rebuilds by accident cannot be compared to the one it claims to extend.
cd "$REPO" || finish 90
HEAD=$(git rev-parse HEAD)
WANT_BIN_REV=${W269_WANT_BIN_REV:-70463ae329adac543de59b36da38112a4044fdeb}
if [ "${W269_REBUILD:-0}" = 1 ]; then
  export PATH=/root/.cargo/bin:$PATH
  export CARGO_TARGET_DIR=/workspace/bench/cargo-target-w268
  export KAYFABE_SHIM_FEATURES=host-isolates
  echo "=== BUILD START $(date -Is) (W269_REBUILD=1) ==="
  scripts/build_qom_shim.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build
  BRC=$?
  echo "=== BUILD RC=$BRC $(date -Is) ==="
  [ $BRC -eq 0 ] || finish 92
  WANT_BIN_REV=$HEAD
  echo "kayfabe-rev:$HEAD" > /workspace/bench/BUILD_REV.txt
fi
echo "=== SCRIPTS HEAD=$HEAD (scripts only; the BINARY's revision is asserted separately) ==="
DIRT=$(git status --porcelain --untracked-files=no)
[ -z "$DIRT" ] || { echo "=== ★ TREE IS DIRTY ==="; echo "$DIRT"; finish 91; }

# ★★★ THE STAMP REQUIREMENT — the anchored form from `w265_run.sh`.
#   ⊘⊘ DO NOT copy `w263`/`w264`'s unbounded `[0-9a-f]*`: it swallows the next `.rodata`
#      literal when that starts with a hex digit, and its message is indistinguishable from
#      the real staleness it guards against.
STAMP=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 \
        | grep -oE 'kayfabe-rev:[0-9a-f]{40}(-dirty)?' | sort -u)
echo "=== STAMP: [$STAMP] REQUIRED: [kayfabe-rev:$WANT_BIN_REV] ==="
NSTAMP=$(printf '%s\n' "$STAMP" | grep -c .)
if [ "$NSTAMP" != "1" ]; then
  echo "=== ★★★ THE EXTRACTION IS THE PROBLEM, NOT THE BINARY: found $NSTAMP stamps, wanted 1."
  echo "===     ⊘ Do NOT read this as a stale build until the extractor is exonerated. ==="
  finish 95
fi
if [ "$STAMP" != "kayfabe-rev:$WANT_BIN_REV" ]; then
  echo "=== ★★★ THE BINARY IS NOT w268's. REFUSING TO BOOT: this rung's whole claim is that"
  echo "===     it measures the SAME binary w268 did, with a guest-side probe as the only"
  echo "===     change. A different binary makes the comparison meaningless. ==="
  finish 93
fi
echo "=== STAMP REQUIREMENT: PASS (no build was performed, and none was needed) ==="

# ★ CONTENT CHECK — the arms this rung CARRIES must be in this binary, or an arm would be a
#   duplicate of the other. ⊘ Only literals that genuinely live in `.rodata`: `why=first` /
#   `why=CHANGED` reach the log through a `{}` PLACEHOLDER and are UNSATISFIABLE by `strings`
#   — that gate refused a correct build on 2026-08-12. A gate's prose is not its assertion.
echo "=== CONTENT CHECK ==="
CC_RC=0
CC_EXTRA=""
# ★★★ When the owner's store witness is built in, its OWN literals are asserted — otherwise a
#     boot that printed no `DOORBELL-STORE` line would be read as "the store never ran" when it
#     means "the witness is not in this binary". Those are opposite conclusions.
[ "${W269_REBUILD:-0}" = 1 ] && CC_EXTRA="DOORBELL-STORE DOORBELL-XLATE DOORBELL-VERB NOT_REACHED_PLACEHOLDER"
CC_EXTRA=${CC_EXTRA/ NOT_REACHED_PLACEHOLDER/}
for s in "KAYFABE_GR_ROUTE" "GR-ROUTE arm=" "GUEST-SEMA arm=" "SEMA-PIN" "PB-PIN" \
         "EXEC-WITNESS ARMED" "GR-CURSOR token=" "SEMA-PAGE-SLOT" "COMPLETION-WATCH" $CC_EXTRA; do
  n=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 | grep -c -- "$s")
  printf '  %-38s = %s\n' "$s" "$n"
  [ "$n" -gt 0 ] || { echo "  ★★★ ZERO — the code under test is NOT in this binary"; CC_RC=1; }
done
[ $CC_RC -eq 0 ] || finish 94

# ⊘ And the probe's own source must be present in THIS checkout, asserted before booting:
#   the hook resolves it from its own directory, and a missing file would produce a hook that
#   prints four "No such file" lines and still exits 0.
for f in scripts/bench/guest_spinprobe.c scripts/bench/cup2_hook_gdbspin.sh; do
  [ -s "$REPO/$f" ] || { echo "=== ★★★ MISSING $f in $REPO — the probe could not run ==="; finish 96; }
  printf '  %-44s md5 %s\n' "$f" "$(md5sum < "$REPO/$f" | cut -d' ' -f1)"
done

export KAYFABE_ISOLATES=real
export KAYFABE_CE_EXECUTOR=host
export NVKVM_RAM_BACKEND=memfd
export KAYFABE_GUEST_RAM=memfd
export POST_CAPTURE_HOOK=$REPO/scripts/bench/cup2_hook_gdbspin.sh
export GQ_TIMEOUT=600
export BOOT_TIMEOUT=180

boot() {
  local tag=$1 fbj=$2 ring=$3 pb=$4 wit=$5 sema=$6 route=$7
  unset KAYFABE_FB_JOIN KAYFABE_GUEST_RING KAYFABE_GUEST_PUSHBUF KAYFABE_PT_WITNESS_EXEC \
        KAYFABE_GUEST_SEMA KAYFABE_GR_ROUTE
  [ "$fbj"   = "-" ] || export KAYFABE_FB_JOIN=$fbj
  [ "$ring"  = "-" ] || export KAYFABE_GUEST_RING=$ring
  [ "$pb"    = "-" ] || export KAYFABE_GUEST_PUSHBUF=$pb
  [ "$wit"   = "-" ] || export KAYFABE_PT_WITNESS_EXEC=$wit
  [ "$sema"  = "-" ] || export KAYFABE_GUEST_SEMA=$sema
  [ "$route" = "-" ] || export KAYFABE_GR_ROUTE=$route
  echo "=== BOOT $tag START $(date -Is) ==="
  echo "    KAYFABE_FB_JOIN=[${KAYFABE_FB_JOIN:-unset}]" \
       "KAYFABE_GUEST_RING=[${KAYFABE_GUEST_RING:-unset}]" \
       "KAYFABE_GUEST_PUSHBUF=[${KAYFABE_GUEST_PUSHBUF:-unset}]" \
       "KAYFABE_PT_WITNESS_EXEC=[${KAYFABE_PT_WITNESS_EXEC:-unset}]" \
       "KAYFABE_GUEST_SEMA=[${KAYFABE_GUEST_SEMA:-unset}]" \
       "KAYFABE_GR_ROUTE=[${KAYFABE_GR_ROUTE:-unset}]"
  timeout 1500 "$REPO/scripts/bench/boot_capture.sh" "$tag"
  echo "=== BOOT $tag RC=$? $(date -Is) ==="
  echo "--- post-boot liveness: pgrep -x qemu-system-x86 = [$(pgrep -x qemu-system-x86 | tr '\n' ' ')] ss2223 = [$(ss -tln 2>/dev/null | grep -c 2223)]"
  echo "--- ENOSPC_LLVM=$(grep -c 'No space left on device\|LLVM ERROR' /workspace/bench/run_${tag}_qemu.log 2>/dev/null || echo '?')"
  echo "--- DISK: $(df -h /workspace | tail -1)"
  echo "--- ARMING AS THE DEVICE SAW IT:"
  for pat in 'FB-JOIN arm=' 'GUEST-RING arm=' 'GUEST-PUSHBUF arm=' 'GUEST-SEMA arm=' 'GR-ROUTE arm='; do
    echo "    $(grep -m1 -o "kayfabe: ${pat}[a-z]*" /workspace/bench/run_${tag}_qemu.log 2>/dev/null || echo "(no ${pat} line)")"
  done
  echo "    $(grep -m1 -o 'EXEC-WITNESS [A-Z]*[^|]*' /workspace/bench/run_${tag}_qemu.log 2>/dev/null | cut -c1-120 || echo '(no EXEC-WITNESS line)')"
  # ★★★ THE ONE VARIABLE, asserted out of the device's own log. A typo that silently disarmed
  #     the route would make the evidence run and its control INDISTINGUISHABLE.
  local wantroute='refuse'; [ "$route" = "passthrough" ] && wantroute='passthrough'
  if grep -q "GR-ROUTE arm=$wantroute" /workspace/bench/run_${tag}_qemu.log 2>/dev/null; then
    echo "    ★ ROUTE-ARM ASSERTION: PASS (saw GR-ROUTE arm=$wantroute)"
  else
    echo "    ★★★ ROUTE-ARM ASSERTION: FAIL — wanted GR-ROUTE arm=$wantroute. ⊘ VOID."
  fi
  for pair in "GUEST-SEMA arm=$sema" "GUEST-PUSHBUF arm=$pb"; do
    if grep -q "$pair" /workspace/bench/run_${tag}_qemu.log 2>/dev/null; then
      echo "    ★ CARRIED-ARM ASSERTION: PASS ($pair)"
    else
      echo "    ★★★ CARRIED-ARM ASSERTION: FAIL — wanted '$pair'. ⊘ VOID."
    fi
  done
  local wantwit='DISARMED'; [ "$wit" = "on" ] && wantwit='ARMED'
  grep -q "EXEC-WITNESS $wantwit" /workspace/bench/run_${tag}_qemu.log 2>/dev/null \
    && echo "    ★ WITNESS-ARM ASSERTION: PASS (EXEC-WITNESS $wantwit)" \
    || echo "    ★★★ WITNESS-ARM ASSERTION: FAIL — wanted EXEC-WITNESS $wantwit. ⊘ VOID."
  # ⊘ The completion plane w268 measured, carried forward as a REGRESSION guard: if the eight
  #   slots stop being written, this rung's whole comparison changes meaning.
  echo "    --- COMPLETION-WATCH OBSERVED = $(grep -c 'COMPLETION-WATCH.*OBSERVED' /workspace/bench/run_${tag}_qemu.log 2>/dev/null || echo '?') | SEMA-PAGE-SLOT = $(grep -c 'SEMA-PAGE-SLOT' /workspace/bench/run_${tag}_qemu.log 2>/dev/null || echo '?')"
  echo "    --- GR-REPORT-SEMAPHORE slots seen (distinct VA):"
  grep -o 'SEMA-PAGE-SLOT va=0x[0-9a-f]* .*kind=GR-REPORT-SEMAPHORE' /workspace/bench/run_${tag}_qemu.log 2>/dev/null \
    | grep -o 'va=0x[0-9a-f]*' | sort -u | tr '\n' ' ' | sed 's/^/      /'; echo
  echo "    --- Xid (IDENTITY, never a count — w265's lesson):"
  grep -o 'Xid.*' /workspace/bench/run_${tag}_hostdmesg.log 2>/dev/null | cut -c1-200 | sort -u | sed 's/^/      /'
  # ★★★★★ THE OWNER'S ITEM 0 — did the store instruction execute at all?
  echo "    --- DOORBELL WITNESS (owner directive): XLATE=$(grep -c 'DOORBELL-XLATE' /workspace/bench/run_${tag}_qemu.log 2>/dev/null) VERB=$(grep -c 'DOORBELL-VERB' /workspace/bench/run_${tag}_qemu.log 2>/dev/null) STORE-WROTE=$(grep -c 'DOORBELL-STORE.*WROTE' /workspace/bench/run_${tag}_qemu.log 2>/dev/null) STORE-NOT-REACHED=$(grep -c 'DOORBELL-STORE.*NOT REACHED' /workspace/bench/run_${tag}_qemu.log 2>/dev/null) STORE-REFUSED=$(grep -c 'DOORBELL-STORE.*STORE ITSELF WAS REFUSED' /workspace/bench/run_${tag}_qemu.log 2>/dev/null)"
  echo "    --- ★★★ XLATE by ENGINE (task #243: \"user-proc GrCompute doorbells never reach it\" — UNTESTED since w261/w262):"
  grep -o 'DOORBELL-XLATE proc=[0-9]* chan=[0-9]* vchid=[0-9]* engine=[A-Za-z]*' /workspace/bench/run_${tag}_qemu.log 2>/dev/null | sed 's/.*\(proc=[0-9]*\).*\(engine=[A-Za-z]*\)/      \1 \2/' | sort | uniq -c | head -12
  echo "    --- ★★★ VERB engine -> host_token (the join between XLATE and STORE):"
  grep -o 'DOORBELL-VERB engine=[A-Za-z]* host_token=0x[0-9a-f]*' /workspace/bench/run_${tag}_qemu.log 2>/dev/null | sort | uniq -c | head -20
  echo "    --- THE PROBE'S OWN HEADLINES:"
  grep -E 'POLLED ADDRESS|SLOT-JOIN|N items =|KIND=|SNAPSHOT at|NO SNAPSHOT|steps actually taken|reached-cuCtxCreate|CUP2_RC=|SPINPROBE_RC=|VALUE AT IT' \
    /workspace/bench/run_${tag}_probe.log 2>/dev/null | sed 's/^/      /' | head -60
}

#     tag           FB_JOIN  GUEST_RING  GUEST_PUSHBUF  PT_WITNESS_EXEC  GUEST_SEMA  GR_ROUTE
boot ${PFX}_refuse  shared   ring        pin            on               pin         -
boot ${PFX}_pass    shared   ring        pin            on               pin         passthrough

echo "=== ARTEFACT SIZES ==="
ls -l /workspace/bench/run_${PFX}_* 2>/dev/null
finish 0
