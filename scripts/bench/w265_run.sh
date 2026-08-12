#!/usr/bin/env bash
# w265 — the POPULATE side. Two boots, ONE variable: `KAYFABE_PT_WITNESS_EXEC`.
#
#   arm   FB_JOIN  GUEST_RING  GUEST_PUSHBUF  PT_WITNESS_EXEC
#   off   shared   ring        pin            (unset)   <= byte-for-byte w264's `pin` arm
#   on    shared   ring        pin            on        <= THE RUNG
#
# ★★★★★ WHY: `w264` located the wall as *"the address table's POPULATE side never learns the
# pushbuffer leaves"* — and the populate source that learns them is BUILT
# (`SharedDoorbell::witness_executor_fb_pages`, shim.rs:6117) and was DISARMED in all four of
# its arms. `w261`/`w262` armed it; `w263_run.sh` and `w264_run.sh` dropped it without saying
# so. Full argument + 18 pre-registered rows: `docs/design/w265_populate_witness_prereg.md`.
#
# ★★ THE DURABLE HALF, and it is why this script is not just `w264_run.sh` with one more line:
# the defect being corrected is A FLAG NOBODY NAMED. So EVERY `KAYFABE_*` the device reads is
# declared per arm — explicitly `export`ed or explicitly `unset`, never inherited — and the
# arming of each is asserted OUT OF THE BOOT'S OWN LOG. A variable this shell exported and a
# variable the device read are two different facts, and w264 proved the gap is real.
#
# ★ START marker and EXIT line so "file exists but has no terminator" is detectable at all
#   (143 = the JOB was SIGTERMed; 124 = the LAUNCHER's ssh expired while the job ran on —
#   opposite meanings arriving as the same word).
OUT=/workspace/w265_run.log
exec >"$OUT" 2>&1
finish() { echo "=== W265 EXIT rc=$1 at $(date -Is) ==="; exit "$1"; }
echo "=== W265 START $(date -Is) pid=$$ ==="

export PATH=/root/.cargo/bin:$PATH
REPO=/workspace/kayfabe_w265
cd "$REPO" || finish 90
HEAD=$(git rev-parse HEAD)
echo "=== HEAD=$HEAD ==="
DIRT=$(git status --porcelain --untracked-files=no)
[ -z "$DIRT" ] || { echo "=== ★ TREE IS DIRTY ==="; echo "$DIRT"; finish 91; }

export CARGO_TARGET_DIR=/workspace/bench/cargo-target-w265
export KAYFABE_SHIM_FEATURES=host-isolates
echo "=== BUILD START $(date -Is) ==="
scripts/build_qom_shim.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build
BRC=$?
echo "=== BUILD RC=$BRC $(date -Is) ==="
[ $BRC -eq 0 ] || finish 92

# ★★★ THE STAMP GATE. The bench silently served a binary built from `862c7c2` for weeks.
#
# ⊘⊘ MEASURED FALSE POSITIVE, 2026-08-12, and the gate is FIXED HERE — `w263_run.sh` and
# `w264_run.sh` still carry the defect, so DO NOT COPY THEIR VERSION OF THIS BLOCK.
#
#   Their pattern is `grep -o 'kayfabe-rev:[0-9a-f]*\(-dirty\)\?'` — UNBOUNDED `*`. Rust `&str`
#   literals are length-prefixed, NOT NUL-terminated, so they pack adjacently in `.rodata` and
#   `strings` cannot separate them. At `24ea98f` the literal following the stamp begins `6…`,
#   so the greedy class swallowed it and the gate reported:
#     STAMP: [kayfabe-rev:…113af1446]  WANT: [kayfabe-rev:…113af144]      <= 41 hex vs 40
#   The build was CORRECT. ★★ The failure is REVISION-DEPENDENT — it fires only when the next
#   literal happens to start with a hex digit, so `w264` passed by luck — and its message is
#   INDISTINGUISHABLE from the real staleness it guards against. A gate that is right 15/16 of
#   the time and lies in the voice of the defect is worse than one that is merely absent.
#
# ⇒ Anchor to EXACTLY 40 hex, and assert the extraction itself before comparing, so a future
#   extraction bug is LOUD rather than arriving disguised as a stale binary.
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
#   ⊘ `EXEC-WITNESS` and `KAYFABE_PT_WITNESS_EXEC` are the two THIS rung turns on; a zero for
#   either means the boot would measure a binary in which the variable under test does nothing.
echo "=== CONTENT CHECK ==="
CC_RC=0
for s in "EXEC-WITNESS ARMED" "EXEC-WITNESS DISARMED" "KAYFABE_PT_WITNESS_EXEC" \
         "VAS-BIND-CENSUS" "PT-DECODE" "PB-PIN" "NOT ONE PAGE RESOLVED IN GUEST RAM"; do
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
  local tag=$1 fbj=$2 ring=$3 pb=$4 wit=$5
  unset KAYFABE_FB_JOIN KAYFABE_GUEST_RING KAYFABE_GUEST_PUSHBUF KAYFABE_PT_WITNESS_EXEC
  [ "$fbj"  = "-" ] || export KAYFABE_FB_JOIN=$fbj
  [ "$ring" = "-" ] || export KAYFABE_GUEST_RING=$ring
  [ "$pb"   = "-" ] || export KAYFABE_GUEST_PUSHBUF=$pb
  [ "$wit"  = "-" ] || export KAYFABE_PT_WITNESS_EXEC=$wit
  echo "=== BOOT $tag START $(date -Is) ==="
  echo "    KAYFABE_FB_JOIN=[${KAYFABE_FB_JOIN:-unset}]" \
       "KAYFABE_GUEST_RING=[${KAYFABE_GUEST_RING:-unset}]" \
       "KAYFABE_GUEST_PUSHBUF=[${KAYFABE_GUEST_PUSHBUF:-unset}]" \
       "KAYFABE_PT_WITNESS_EXEC=[${KAYFABE_PT_WITNESS_EXEC:-unset}]"
  timeout 900 "$REPO/scripts/bench/boot_capture.sh" "$tag"
  echo "=== BOOT $tag RC=$? $(date -Is) ==="
  # ⊘ pgrep -x qemu-system-x86 (comm truncates at 15); NOT -f (it matches the asker).
  echo "--- post-boot liveness: pgrep -x qemu-system-x86 = [$(pgrep -x qemu-system-x86 | tr '\n' ' ')] ss2223 = [$(ss -tln 2>/dev/null | grep -c 2223)]"
  # ⚠ From the SAME invocation that produced the status, per CLAUDE.md.
  echo "--- ENOSPC_LLVM=$(grep -c 'No space left on device\|LLVM ERROR' /workspace/bench/run_${tag}_qemu.log 2>/dev/null || echo '?')"
  # ★★ THE ARMING AS THE DEVICE SAW IT. Four flags, four assertions, read out of the boot's
  #    own log. `EXEC-WITNESS` is the one w264 could not have caught, because nothing asked.
  echo "--- ARMING AS THE DEVICE SAW IT:"
  for pat in 'FB-JOIN arm=' 'GUEST-RING arm=' 'GUEST-PUSHBUF arm='; do
    echo "    $(grep -m1 -o "kayfabe: ${pat}[a-z]*" /workspace/bench/run_${tag}_qemu.log 2>/dev/null || echo "(no ${pat} line)")"
  done
  echo "    $(grep -m1 -o 'EXEC-WITNESS [A-Z]*[^|]*' /workspace/bench/run_${tag}_qemu.log 2>/dev/null | cut -c1-120 || echo '(no EXEC-WITNESS line)')"
  # ★★★ THE ASSERTION, not a print. The whole rung is this one variable; an arm that took the
  #     wrong one is not a data point, it is a duplicate of the other arm.
  local want='DISARMED'; [ "$wit" = "on" ] && want='ARMED'
  if grep -q "EXEC-WITNESS $want" /workspace/bench/run_${tag}_qemu.log 2>/dev/null; then
    echo "    ★ WITNESS-ARM ASSERTION: PASS (saw EXEC-WITNESS $want, as this arm requires)"
  else
    echo "    ★★★ WITNESS-ARM ASSERTION: FAIL — arm '$tag' wanted EXEC-WITNESS $want and the"
    echo "        device did not say so. ⊘ This arm's numbers are VOID; do not read them."
  fi
}

#     tag       FB_JOIN  GUEST_RING  GUEST_PUSHBUF  PT_WITNESS_EXEC
boot w265_off   shared   ring        pin            -
boot w265_on    shared   ring        pin            on

echo "=== ARTEFACT SIZES ==="
ls -l /workspace/bench/run_w265_* 2>/dev/null
finish 0
