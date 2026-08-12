#!/usr/bin/env bash
# w274 — ONE BOOT, TWO MEASUREMENTS, NO DEVICE VARIABLE.
#
# The arming is `w271_pin`'s, byte for byte, and nothing in the device changes. That is
# deliberate: this rung measures the GUEST, not our device, so the device must be the
# constant. ⇒ every number here is directly comparable to `w271_pin`'s.
#
#   arm  FB_JOIN  GUEST_RING  GUEST_PUSHBUF  PT_WITNESS_EXEC  GUEST_SEMA  GR_ROUTE     GUEST_OPERAND
#   pin  shared   ring        pin            on               pin         passthrough  pin
#
# ## The two measurements
#
#  1. ★★★★★ THE SPIN PROOF. `/proc/<tid>/syscall` across EVERY thread, 24 samples, plus
#     utime/stime integrals and voluntary-context-switch counts, plus the polled semaphore
#     words each sample. Two samples cannot tell a spin from a coincidence; this is a
#     distribution. Refutations pre-registered in the hook's own header.
#  2. ★★★ THE IOCTL DIFFERENTIAL, re-run. `tests/mode2/nvdiff` is stale by ~60 commits — its
#     standing headline predates `w210`, where we SERVED `0x20801702` and `cuCtxCreate`
#     stopped returning at all. The host reference is captured on THIS BOX (single GA106,
#     open 580.159.04 — the same chip and driver the guest targets) rather than reused from
#     the five-GPU closed-driver rig, so the environmental divergences that dominated the old
#     diff by index are gone before the diff instead of ranked around after it.
#
# ⊘ Phase 2 runs strictly AFTER cup2 exits. Two CUDA processes at once would make the /proc
#   sampling report on a machine other than the one it measured.
#
# ★ START marker and EXIT line so "file exists but has no terminator" is detectable at all
#   (143 = the JOB was SIGTERMed; 124 = the LAUNCHER's ssh expired while the job ran on).
PFX=${W274B_TAG_PREFIX:-w274b}
OUT=/workspace/${PFX}_run.log
exec >"$OUT" 2>&1
finish() { echo "=== W274B EXIT rc=$1 at $(date -Is) ==="; exit "$1"; }
echo "=== W274B START $(date -Is) pid=$$ ==="

export PATH=/root/.cargo/bin:$PATH
REPO=${KAYFABE_REPO:-/workspace/kayfabe_w274}
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
# ⚠ From the SAME invocation: disk is the failure that reads as a code error.
echo "=== BUILD ENOSPC/LLVM = $(grep -c 'No space left on device\|LLVM ERROR' "$OUT" 2>/dev/null) | df: $(df -h /workspace | tail -1) ==="

# ★★★ THE STAMP GATE, anchored to exactly 40 hex. An unbounded [0-9a-f]* swallows the next
# .rodata literal and its message is indistinguishable from the real thing.
STAMP=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 2>/dev/null \
        | grep -oE 'kayfabe-rev:[0-9a-f]{40}' | head -1 | cut -d: -f2)
echo "=== STAMP=$STAMP HEAD=$HEAD ==="
[ "$STAMP" = "$HEAD" ] || { echo "=== ★★★ STAMP GATE FAIL: the binary is not this HEAD ==="; finish 93; }

export KAYFABE_ISOLATES=real
export KAYFABE_CE_EXECUTOR=host
export NVKVM_RAM_BACKEND=memfd
export KAYFABE_GUEST_RAM=memfd
export POST_CAPTURE_HOOK=$REPO/scripts/bench/nvdiff_hook.sh
export NVDIFF_SRC_DIR=${NVDIFF_SRC_DIR:-/workspace/nvdiff_src}
export NVD_GUEST_OUT=/workspace/bench/nvdiff_guest_ce
export NVD_HANG_WAIT=90
export GQ_TIMEOUT=900
export BOOT_TIMEOUT=180

for f in scripts/bench/nvdiff_hook.sh; do
  [ -f "$f" ] || { echo "=== ★★★ MISSING $f — the probe cannot run ==="; finish 96; }
done
for f in nvdiff_shim.c nvd_prog.c nvd_capture.sh nvd_cuda_min.h uvm_sizes.h; do
  [ -f "$NVDIFF_SRC_DIR/$f" ] || { echo "=== ★★★ MISSING $NVDIFF_SRC_DIR/$f — phase 2 cannot run ==="; finish 97; }
done

export KAYFABE_FB_JOIN=shared
export KAYFABE_GUEST_RING=ring
export KAYFABE_GUEST_PUSHBUF=pin
export KAYFABE_PT_WITNESS_EXEC=on
export KAYFABE_GUEST_SEMA=pin
export KAYFABE_GR_ROUTE=passthrough
export KAYFABE_GUEST_OPERAND=pin

TAG=${PFX}_pin
echo "=== BOOT $TAG START $(date -Is) ==="
timeout 1800 "$REPO/scripts/bench/boot_capture.sh" "$TAG"
echo "=== BOOT $TAG RC=$? $(date -Is) ==="

Q=/workspace/bench/run_${TAG}_qemu.log
P=/workspace/bench/run_${TAG}_probe.log
D=/workspace/bench/run_${TAG}_hostdmesg.log

echo "--- post-boot liveness: pgrep -x qemu-system-x86 = [$(pgrep -x qemu-system-x86 | tr '\n' ' ')] ss2223 = [$(ss -tln 2>/dev/null | grep -c 2223)]"
echo "--- ENOSPC_LLVM=$(grep -c 'No space left on device\|LLVM ERROR' "$Q" 2>/dev/null || echo '?')"

# ★★ THE ARMING AS THE DEVICE SAW IT — asserted, not printed, so a mis-armed boot is not a
#    data point that looks like one.
for pair in 'FB-JOIN arm=shared' 'GUEST-RING arm=ring' 'GUEST-PUSHBUF arm=pin' \
            'GUEST-SEMA arm=pin' 'GR-ROUTE arm=passthrough' 'GUEST-OPERAND arm=pin'; do
  grep -q "kayfabe: $pair" "$Q" 2>/dev/null \
    && echo "    ★ ARM ASSERTION PASS: $pair" \
    || echo "    ★★★ ARM ASSERTION FAIL: wanted '$pair'. ⊘ THIS BOOT IS VOID for comparison."
done

# ★ Comparability to w271_pin, as numbers rather than as a claim.
echo "--- COMPARABILITY TO w271_pin (the device is unchanged; these should track):"
echo "    DOORBELL-XLATE  = $(grep -c 'DOORBELL-XLATE' "$Q" 2>/dev/null)   (w271_pin: 88)"
echo "    OPERAND-PIN     = $(grep -c 'OPERAND-PIN' "$Q" 2>/dev/null)   (w271_pin: 88)"
echo "    doorbells served/forwarded = [$(grep -o '[0-9]* doorbells served, [0-9]* of them forwarded' "$Q" 2>/dev/null | tail -1)]   (w271_pin: 201 / 12)"

# ★★★★★ Xid BY IDENTITY, NEVER BY COUNT. ⚠ engine and channel are ONE measurement reported
#        twice (kchannelGetDebugTag returns (runlistId<<24)|ChID) — do not tally them as two.
echo "--- ★★★★★ Xid IDENTITY:"
grep -o 'Xid.*' "$D" 2>/dev/null | cut -c1-220 | sort -u | sed 's/^/      /'
echo "    distinct fault ADDRESSES = [$(grep -oE 'faulted @ 0x[0-9a-f_]+' "$D" 2>/dev/null | sort -u | tr '\n' ' ')]"
echo "    ENGINE/CLIENT pairs      = [$(grep -oE 'ENGINE [A-Z0-9_]+ HUBCLIENT_[A-Z0-9_]+' "$D" 2>/dev/null | sort -u | tr '\n' ' ')]"
echo "    ACCESS types             = [$(grep -oE 'ACCESS_TYPE_[A-Z_]+' "$D" 2>/dev/null | sort -u | tr '\n' ' ')]"
echo "    FAULT types              = [$(grep -oE 'FAULT_[A-Z]+' "$D" 2>/dev/null | sort -u | tr '\n' ' ')]"
echo "    ⊘ HOST_DMESG bytes=$(stat -c %s "$D" 2>/dev/null || echo '?') — a ZERO is only 'no fault' if the watermark says so"

# ★★★ THE ANCHORED CUP2_RC. `grep 'CUP2_RC=[0-9]*'` matches GCC_CUP2_RC=0 and has reported
#     the campaign's headline success value on a hanging arm. Anchor it.
echo "--- ★★★ CUP2_RC (anchored):"
grep -h '^CUP2_RC=' "$P" 2>/dev/null | sed 's/^/      /'
echo "    ⊘ unanchored, for contrast: [$(grep -oh 'CUP2_RC=[0-9]*' "$P" 2>/dev/null | tr '\n' ' ')]"

# ★★★★★ THE SPIN DISTRIBUTION — computed here, from the boot's own probe log.
echo "--- ★★★★★ SPIN DISTRIBUTION (per thread: how many samples were USERSPACE):"
awk '/^S i=/ {
       tid=""; st=""; sc="";
       for (i=1;i<=NF;i++) {
         if ($i ~ /^tid=/)   { tid=substr($i,5) }
         if ($i ~ /^state=/) { st=substr($i,7)  }
       }
       n[tid]++; state[tid"/"st]++;
       if ($0 ~ /syscall=\[running\]/) run[tid]++; else blk[tid]++;
     }
     END {
       for (t in n)
         printf("      tid=%s samples=%d USERSPACE(running)=%d IN-SYSCALL=%d\n",
                t, n[t], run[t]+0, blk[t]+0);
     }' "$P" 2>/dev/null | sort
echo "    ⊘ if every count is 0 the sampler did not run; that is not 'no spin'."
echo "--- per-thread syscall values actually seen (a blocked thread is NAMED, not counted):"
grep -oE 'tid=[0-9]+ state=[A-Za-z]+ wchan=\[[^]]*\] syscall=\[[^]]*\]' "$P" 2>/dev/null \
  | sed -E 's/tid=[0-9]+ //' | sort | uniq -c | sort -rn | head -20 | sed 's/^/      /'
echo "--- utime/stime integrals (BASE -> FIN). ★ stime that does not move = no syscall cost:"
grep -E '^(BASE|FIN) tid=' "$P" 2>/dev/null | sed 's/^/      /'
echo "--- the polled semaphore words, first and last sample (⚠ 'never moves' and 'moves but"
echo "    never reaches the awaited value' are DIFFERENT BUGS):"
grep -E '^SEM i=1 ' "$P" 2>/dev/null | sed 's/^/      FIRST /' | head -16
grep -E '^SEM i=' "$P" 2>/dev/null | tail -16 | sed 's/^/      LAST  /'
echo "    distinct semaphore-region snapshots = $(grep -E '^SEM i=' "$P" 2>/dev/null | sed 's/^SEM i=[0-9]* t=[0-9.]* //' | sort -u | wc -l)"

echo "--- ★★★ NVDIFF GUEST CAPTURE:"
echo "    lines = $(wc -l < /workspace/bench/nvdiff_guest_ce_r1.jsonl 2>/dev/null || echo 0)"
echo "    stdout:"; sed 's/^/      /' /workspace/bench/nvdiff_guest_ce_r1.stdout 2>/dev/null | head -20
finish 0
