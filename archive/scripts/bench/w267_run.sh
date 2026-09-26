#!/usr/bin/env bash
# w267 — READ THE 4 KiB PAGE. Two boots, ONE variable: `KAYFABE_GUEST_SEMA`.
#
#   arm   FB_JOIN  GUEST_RING  GUEST_PUSHBUF  PT_WITNESS_EXEC  GUEST_SEMA
#   off   shared   ring        pin            on               (unset)  <= byte-for-byte w266's `off`
#   on    shared   ring        pin            on               pin      <= w266's `on`, INSTRUMENTED
#
# ★★★★★ WHY: `w266` pinned the completion page and the host GPU's eight `Xid 31 … @
# 0x2_0440f000 ACCESS_TYPE_VIRT_WRITE` went to ZERO — while `COMPLETION-WATCH` stayed
# `NOT-OBSERVED` on all eight declared addresses. Zero faults AND zero completions at once. That
# is ambiguous between (a) a write that landed at a slot nobody watches and (b) a write never
# attempted, and `w266` could not separate them because NOTHING IN THE TREE READ MORE THAN FOUR
# BYTES OF GUEST RAM. This rung adds the reader. Full argument, six named readings of the dump
# and the CE-address arithmetic: `docs/design/w267_read_the_page_prereg.md`.
#
# ⊘⊘ THE VARIABLE IS UNCHANGED FROM `w266` ON PURPOSE. Both new instruments (`SEMA-PAGE`, and
# `push_headers` printing every argument) are on BOTH arms — they are instruments, not the
# variable. ★ That makes the `off` arm the negative control the brief did not ask for and the
# sharpest row here: on `off` the engine FAULTS instead of writing, so content that appears only
# on `on` is engine-written by elimination.
#
# ★★ THE `off` ARM IS A CROSS-REVISION GUARD, not only a control: it is `w266_off` re-run at a
# new revision. If its numbers move, the comparison is void and the grader says so first.
#
# ★★ EVERY `KAYFABE_*` the device reads is declared per arm — explicitly `export`ed or
# explicitly `unset`, never inherited — and the arming of each is asserted OUT OF THE BOOT'S OWN
# LOG. A variable this shell exported and a variable the device read are two different facts,
# and `w264` proved the gap is real.
#
# ★ START marker and EXIT line so "file exists but has no terminator" is detectable at all
#   (143 = the JOB was SIGTERMed; 124 = the LAUNCHER's ssh expired while the job ran on —
#   opposite meanings arriving as the same word).
OUT=/workspace/w267_run.log
exec >"$OUT" 2>&1
finish() { echo "=== W267 EXIT rc=$1 at $(date -Is) ==="; exit "$1"; }
echo "=== W267 START $(date -Is) pid=$$ ==="

export PATH=/root/.cargo/bin:$PATH
REPO=/workspace/kayfabe_w267
cd "$REPO" || finish 90
HEAD=$(git rev-parse HEAD)
echo "=== HEAD=$HEAD ==="
DIRT=$(git status --porcelain --untracked-files=no)
[ -z "$DIRT" ] || { echo "=== ★ TREE IS DIRTY ==="; echo "$DIRT"; finish 91; }

export CARGO_TARGET_DIR=/workspace/bench/cargo-target-w267
export KAYFABE_SHIM_FEATURES=host-isolates
echo "=== BUILD START $(date -Is) ==="
scripts/build_qom_shim.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build
BRC=$?
echo "=== BUILD RC=$BRC $(date -Is) ==="
[ $BRC -eq 0 ] || finish 92

# ★★★ THE STAMP GATE — the FIXED form (anchored to exactly 40 hex), from `w265_run.sh`.
# ⊘⊘ DO NOT COPY `w263_run.sh` / `w264_run.sh`'s version: their unbounded `[0-9a-f]*` swallows
#    the next `.rodata` literal when it starts with a hex digit, and the resulting message is
#    INDISTINGUISHABLE from the real staleness it guards against.
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
#   ⊘ `SEMA-PIN` and `KAYFABE_GUEST_SEMA` are the two THIS rung turns on; a zero for either
#   means the boot would measure a binary in which the variable under test does nothing.
echo "=== CONTENT CHECK ==="
CC_RC=0
# ⊘ The FIRST FIVE are THIS rung's own code. A zero for any of them means the boot would
#   measure a binary in which the reader under test does not exist, and every `SEMA-PAGE` row
#   would be absent BY CONSTRUCTION — indistinguishable, in the log, from a page nobody looked
#   at. That is the zero-byte-artefact trap wearing a different hat.
for s in "SEMA-PAGE seq=" "SEMA-PAGE-SLOT" "SEMA-PAGE-ZERO" "SEMA-PAGE-READER stopped" \
         "LISTING-BOUND" \
         "SEMA-PIN" "KAYFABE_GUEST_SEMA" "GUEST-SEMA arm=" "NO PAGE TO PIN" \
         "SEMAPHORE RUN(S) PLACED" "SEMA-TABLE:" "EXEC-WITNESS ARMED" "PB-PIN" "COMPLETION-WATCH" \
         "VAS-BIND-CENSUS" "PT-DECODE"; do
  n=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 | grep -c -- "$s")
  printf '  %-38s = %s\n' "$s" "$n"
  [ "$n" -gt 0 ] || { echo "  ★★★ ZERO — the code under test is NOT in this binary"; CC_RC=1; }
done
[ $CC_RC -eq 0 ] || finish 94

export KAYFABE_ISOLATES=real
export KAYFABE_CE_EXECUTOR=host
export NVKVM_RAM_BACKEND=memfd
export KAYFABE_GUEST_RAM=memfd
export POST_CAPTURE_HOOK=$REPO/scripts/bench/cup2_hook_w232.sh
export GQ_TIMEOUT=300
export BOOT_TIMEOUT=180

# ⊘ EVERY per-arm variable is unset first and set only if this arm names it, so an arm can
#   never inherit the previous arm's arming.
boot() {
  local tag=$1 fbj=$2 ring=$3 pb=$4 wit=$5 sema=$6
  unset KAYFABE_FB_JOIN KAYFABE_GUEST_RING KAYFABE_GUEST_PUSHBUF KAYFABE_PT_WITNESS_EXEC \
        KAYFABE_GUEST_SEMA
  [ "$fbj"  = "-" ] || export KAYFABE_FB_JOIN=$fbj
  [ "$ring" = "-" ] || export KAYFABE_GUEST_RING=$ring
  [ "$pb"   = "-" ] || export KAYFABE_GUEST_PUSHBUF=$pb
  [ "$wit"  = "-" ] || export KAYFABE_PT_WITNESS_EXEC=$wit
  [ "$sema" = "-" ] || export KAYFABE_GUEST_SEMA=$sema
  echo "=== BOOT $tag START $(date -Is) ==="
  echo "    KAYFABE_FB_JOIN=[${KAYFABE_FB_JOIN:-unset}]" \
       "KAYFABE_GUEST_RING=[${KAYFABE_GUEST_RING:-unset}]" \
       "KAYFABE_GUEST_PUSHBUF=[${KAYFABE_GUEST_PUSHBUF:-unset}]" \
       "KAYFABE_PT_WITNESS_EXEC=[${KAYFABE_PT_WITNESS_EXEC:-unset}]" \
       "KAYFABE_GUEST_SEMA=[${KAYFABE_GUEST_SEMA:-unset}]"
  timeout 900 "$REPO/scripts/bench/boot_capture.sh" "$tag"
  echo "=== BOOT $tag RC=$? $(date -Is) ==="
  # ⊘ pgrep -x qemu-system-x86 (comm truncates at 15); NOT -f (it matches the asker).
  echo "--- post-boot liveness: pgrep -x qemu-system-x86 = [$(pgrep -x qemu-system-x86 | tr '\n' ' ')] ss2223 = [$(ss -tln 2>/dev/null | grep -c 2223)]"
  # ⚠ From the SAME invocation that produced the status, per CLAUDE.md.
  echo "--- ENOSPC_LLVM=$(grep -c 'No space left on device\|LLVM ERROR' /workspace/bench/run_${tag}_qemu.log 2>/dev/null || echo '?')"
  # ★★ THE ARMING AS THE DEVICE SAW IT. Five flags, read out of the boot's own log.
  echo "--- ARMING AS THE DEVICE SAW IT:"
  for pat in 'FB-JOIN arm=' 'GUEST-RING arm=' 'GUEST-PUSHBUF arm=' 'GUEST-SEMA arm='; do
    echo "    $(grep -m1 -o "kayfabe: ${pat}[a-z]*" /workspace/bench/run_${tag}_qemu.log 2>/dev/null || echo "(no ${pat} line)")"
  done
  echo "    $(grep -m1 -o 'EXEC-WITNESS [A-Z]*[^|]*' /workspace/bench/run_${tag}_qemu.log 2>/dev/null | cut -c1-120 || echo '(no EXEC-WITNESS line)')"
  # ★★★ THE ASSERTIONS, not prints. The whole rung is ONE variable; an arm that took the wrong
  #     one is not a data point, it is a duplicate of the other arm.
  local wantsema='off'; [ "$sema" = "pin" ] && wantsema='pin'
  if grep -q "GUEST-SEMA arm=$wantsema" /workspace/bench/run_${tag}_qemu.log 2>/dev/null; then
    echo "    ★ SEMA-ARM ASSERTION: PASS (saw GUEST-SEMA arm=$wantsema, as this arm requires)"
  else
    echo "    ★★★ SEMA-ARM ASSERTION: FAIL — arm '$tag' wanted GUEST-SEMA arm=$wantsema and the"
    echo "        device did not say so. ⊘ This arm's numbers are VOID; do not read them."
  fi
  # ⚠ The CARRIED arms are asserted too. `w266` is only a valid predecessor if leg 4 and the
  #   witness are armed on BOTH arms here; a silently dropped flag is exactly what w264 hit.
  local wantwit='DISARMED'; [ "$wit" = "on" ] && wantwit='ARMED'
  if grep -q "EXEC-WITNESS $wantwit" /workspace/bench/run_${tag}_qemu.log 2>/dev/null; then
    echo "    ★ WITNESS-ARM ASSERTION: PASS (saw EXEC-WITNESS $wantwit)"
  else
    echo "    ★★★ WITNESS-ARM ASSERTION: FAIL — wanted EXEC-WITNESS $wantwit. ⊘ VOID."
  fi
  if grep -q "GUEST-PUSHBUF arm=$pb" /workspace/bench/run_${tag}_qemu.log 2>/dev/null; then
    echo "    ★ PUSHBUF-ARM ASSERTION: PASS (saw GUEST-PUSHBUF arm=$pb)"
  else
    echo "    ★★★ PUSHBUF-ARM ASSERTION: FAIL — wanted GUEST-PUSHBUF arm=$pb. ⊘ VOID."
  fi
  # ⊘ And the CONTROL's own expected result, asserted as an ABSENCE with a number rather than
  #   read as one: a disarmed run must print ZERO `SEMA-PIN` lines.
  echo "    --- SEMA-PIN line count = $(grep -c 'SEMA-PIN' /workspace/bench/run_${tag}_qemu.log 2>/dev/null || echo '?') (control wants 0, rung wants >0)"
  # ★★★★★ THE READER'S OWN NON-VACUITY, asserted on BOTH arms — because this rung's whole
  #   product is a dump, and "the dump says the page is empty" and "the dump never ran" are the
  #   two answers it exists to separate. `SEMA-PAGE-READER stopped` prints unconditionally when
  #   the observer thread exits, so its ABSENCE means the thread never got there.
  local npage nread
  npage=$(grep -c 'SEMA-PAGE seq=' /workspace/bench/run_${tag}_qemu.log 2>/dev/null)
  nread=$(grep -c 'SEMA-PAGE-READER stopped' /workspace/bench/run_${tag}_qemu.log 2>/dev/null)
  echo "    --- SEMA-PAGE dumps = $npage | SEMA-PAGE-READER stopped = $nread"
  if [ "${npage:-0}" -gt 0 ] && [ "${nread:-0}" -gt 0 ]; then
    echo "    ★ PAGE-READER ASSERTION: PASS (the reader ran AND terminated with its own tally)"
  else
    echo "    ★★★ PAGE-READER ASSERTION: FAIL — dumps=$npage stopped=$nread. ⊘ Do NOT read an"
    echo "        absent SEMA-PAGE row as an empty page: it is a statement about the instrument."
  fi
  # ⊘ And the reader's own numbers, quoted so a suppressed-dump count is never invisible.
  echo "    $(grep -m1 -o 'SEMA-PAGE-READER stopped[^(]*' /workspace/bench/run_${tag}_qemu.log 2>/dev/null || echo '(no reader tally)')"
}

#     tag       FB_JOIN  GUEST_RING  GUEST_PUSHBUF  PT_WITNESS_EXEC  GUEST_SEMA
boot w267_off   shared   ring        pin            on               -
boot w267_on    shared   ring        pin            on               pin

echo "=== ARTEFACT SIZES ==="
ls -l /workspace/bench/run_w267_* 2>/dev/null
finish 0
