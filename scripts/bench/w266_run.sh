#!/usr/bin/env bash
# w266 — LEG 5, the COMPLETION plane. Two boots, ONE variable: `KAYFABE_GUEST_SEMA`.
#
#   arm   FB_JOIN  GUEST_RING  GUEST_PUSHBUF  PT_WITNESS_EXEC  GUEST_SEMA
#   off   shared   ring        pin            on               (unset)  <= byte-for-byte w265's `on`
#   on    shared   ring        pin            on               pin      <= THE RUNG
#
# ★★★★★ WHY: `w265` armed leg 4 and the host GPU's eight `Xid 31` CHANGED IDENTITY —
#   off: ENGINE CE3_PBDMA0 HUBCLIENT_ESC @ 8 pushbuffer VAs  ACCESS_TYPE_VIRT_READ
#   on:  ENGINE CE3        HUBCLIENT_CE1 @ 0x2_0440f000      ACCESS_TYPE_VIRT_WRITE
# The copy engine began EXECUTING the guest's methods and cannot WRITE the completion. That one
# address is the page holding all eight `SET_REPORT_SEMAPHORE` targets (0x20440ff80 …
# 0x20440fff0, 16-byte stride, every one site=GuestRam). This rung points the SAME pin that
# placed eight pushbuffer runs at that ONE page. Full argument + 22 pre-registered rows:
# `docs/design/w266_completion_pin_prereg.md`.
#
# ★★ THE `off` ARM IS A CROSS-REVISION GUARD, not only a control: it is `w265_on` re-run at a
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
OUT=/workspace/w266_run.log
exec >"$OUT" 2>&1
finish() { echo "=== W266 EXIT rc=$1 at $(date -Is) ==="; exit "$1"; }
echo "=== W266 START $(date -Is) pid=$$ ==="

export PATH=/root/.cargo/bin:$PATH
REPO=/workspace/kayfabe_w266
cd "$REPO" || finish 90
HEAD=$(git rev-parse HEAD)
echo "=== HEAD=$HEAD ==="
DIRT=$(git status --porcelain --untracked-files=no)
[ -z "$DIRT" ] || { echo "=== ★ TREE IS DIRTY ==="; echo "$DIRT"; finish 91; }

export CARGO_TARGET_DIR=/workspace/bench/cargo-target-w266
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
for s in "SEMA-PIN" "KAYFABE_GUEST_SEMA" "GUEST-SEMA arm=" "NO PAGE TO PIN" \
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
  # ⚠ The CARRIED arms are asserted too. `w265` is only a valid predecessor if leg 4 and the
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
}

#     tag       FB_JOIN  GUEST_RING  GUEST_PUSHBUF  PT_WITNESS_EXEC  GUEST_SEMA
boot w266_off   shared   ring        pin            on               -
boot w266_on    shared   ring        pin            on               pin

echo "=== ARTEFACT SIZES ==="
ls -l /workspace/bench/run_w266_* 2>/dev/null
finish 0
