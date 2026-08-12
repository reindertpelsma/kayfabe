#!/usr/bin/env bash
# ★★★ w274 POST_CAPTURE_HOOK — capture the GUEST half of the ioctl differential.
#
# ## ⊘⊘ WHY THIS IS A SEPARATE HOOK AND NOT PHASE 2 OF `cup2_hook_procspin.sh`
#
# [measured 2026-08-12, boot `w274_pin`] Phase 2 there never ran. When `cup2` hit its 180 s
# timeout and was torn down, QEMU **aborted (core dumped)** and the guest went away:
#
#   thread '<unnamed>' panicked at crates/kayfabe-util/src/lockwitness.rs:152:5:
#   R1 no-blocking-under-lock violation (l1_concurrency.md §3.3): munmap (dropping a host
#   mapping) while holding rank(s) [0]
#   ... 19: kayfabe_shim_regs_write ... thread caused non-unwinding panic. aborting.
#
# ⇒ Our own R1 witness fired — correctly — inside an `extern "C"` frame that cannot unwind.
# **Anything scheduled after a hung CUDA process is torn down is unreachable by construction.**
#
# ## THE FIX IS ORDERING, NOT A RETRY
#
# The shim appends each record to an `O_APPEND` fd **as the ioctl happens**, not at teardown.
# ⇒ The trace is complete and readable WHILE the workload is still hung. So this hook copies
# it out DURING the hang and never depends on the process exiting at all.
#
# ⊘ That also makes the capture robust to the thing it is measuring: `nvd_prog` will hang at
# `cuCtxCreate` exactly as `cup2` does. **The partial trace up to the hang IS the
# measurement** — the divergence point is what we are after, and a workload that completed
# would not have one.
#
#   usage: POST_CAPTURE_HOOK=scripts/bench/nvdiff_hook.sh scripts/bench/boot_capture.sh <tag>
#   ⚠ needs GQ_TIMEOUT >= 600 and NVDIFF_SRC_DIR set.
set -uo pipefail
G="$(cd "$(dirname "$0")" && pwd)/gssh_nv"
SRC=${NVDIFF_SRC_DIR:-/workspace/nvdiff_src}
OUTBASE=${NVD_GUEST_OUT:-/workspace/bench/nvdiff_guest_ce}
HANG_WAIT=${NVD_HANG_WAIT:-90}

die() { echo "★ w274 nvdiff hook FAILED: $*"; exit 2; }

echo "=== sources (md5 — a run cannot silently be a different copy) ==="
for f in nvdiff_shim.c nvd_prog.c nvd_capture.sh nvd_cuda_min.h uvm_sizes.h; do
  [ -f "$SRC/$f" ] || die "missing $SRC/$f"
  printf '    %-20s md5 %s\n' "$f" "$(md5sum < "$SRC/$f" | cut -d' ' -f1)"
done

echo "=== push + build the shim and the workload IN THE GUEST ==="
$G 'mkdir -p /tmp/nvdiff'
for f in nvdiff_shim.c nvd_prog.c nvd_capture.sh nvd_cuda_min.h uvm_sizes.h; do
  $G "cat > /tmp/nvdiff/$f" < "$SRC/$f" || die "could not push $f"
done
# ⚠ NVD_MIN_CUDA=1 on BOTH sides. The host reference on `vh` was built with the bundled
#   stand-in header because that box has no CUDA toolkit; building the guest against a real
#   cuda.h would make the two BINARIES differ, and the binary is supposed to be the constant.
#   The symbol-binding gate inside nvd_capture.sh refuses the build if the seven versioned
#   entry points bind v1 instead of _v2.
$G 'cd /tmp/nvdiff && NVD_MIN_CUDA=1 bash nvd_capture.sh /tmp/nvdiff/out ce 0 \
      > /tmp/nvdiff/build.log 2>&1; echo BUILD_PHASE_RC=$?
    echo "--- ENOSPC check from the SAME invocation ---"
    grep -c "No space left on device\|LLVM ERROR" /tmp/nvdiff/build.log
    echo "--- symbol-binding gate ---"
    grep -E "^   ok|NOT BOUND|FATAL|^== build" /tmp/nvdiff/build.log'
$G 'test -x /tmp/nvdiff/out/nvd_prog && test -f /tmp/nvdiff/out/nvdiff_shim.so' \
  || die "the shim or the workload did not build in the guest"

echo "=== run nvd_prog DETACHED under the shim (it will hang at cuCtxCreate; that is the point) ==="
$G 'cat > /tmp/nvdiff/run.sh' <<'GUESTEOF'
#!/bin/sh
rm -f /tmp/nvdiff/out/ce_r1.jsonl /tmp/nvdiff/out/ce_r1.stdout /tmp/nvdiff/run.rc
echo "STARTED $(date -Is)" > /tmp/nvdiff/run.started
setsid sh -c 'cd /tmp/nvdiff && NVDIFF_OUT=/tmp/nvdiff/out/ce_r1.jsonl NVDIFF_MAXBUF=65536 \
     LD_PRELOAD=/tmp/nvdiff/out/nvdiff_shim.so timeout 240 ./out/nvd_prog ce \
     > /tmp/nvdiff/out/ce_r1.stdout 2>&1; echo $? > /tmp/nvdiff/run.rc' \
     </dev/null >/dev/null 2>&1 &
sleep 1
echo "LAUNCHED pid=$(pgrep -x nvd_prog | head -1)"
GUESTEOF
$G 'sh /tmp/nvdiff/run.sh'

echo "=== waiting ${HANG_WAIT}s, then EXTRACTING WHILE IT IS STILL HUNG ==="
echo "    ⊘ the extraction does NOT wait for the process to exit: a torn-down CUDA process"
echo "      takes the whole VM down with it (R1 witness, measured this boot)."
sleep "$HANG_WAIT"

$G 'echo "--- is it still running? (bracket trick; a bare pgrep -f matches the asker) ---"
    pgrep -x nvd_prog | tr "\n" " "; echo
    echo "--- did it finish early? ---"; cat /tmp/nvdiff/run.rc 2>/dev/null || echo "NO RC — still in flight"
    echo "--- record count so far ---"; wc -l < /tmp/nvdiff/out/ce_r1.jsonl 2>/dev/null || echo 0
    echo "--- stdout so far (the last ok line names the last call that COMPLETED) ---"
    cat /tmp/nvdiff/out/ce_r1.stdout 2>/dev/null'

echo "=== pulling the capture out ==="
$G 'cat /tmp/nvdiff/out/ce_r1.jsonl 2>/dev/null'  > "${OUTBASE}_r1.jsonl"
$G 'cat /tmp/nvdiff/out/ce_r1.stdout 2>/dev/null' > "${OUTBASE}_r1.stdout"
$G 'cat /tmp/nvdiff/out/env_ce.txt 2>/dev/null'   > "${OUTBASE}_r1.env"
LINES=$(wc -l < "${OUTBASE}_r1.jsonl" 2>/dev/null || echo 0)
echo "    guest jsonl lines = $LINES"
# ★ ASSERT the capture is real. An existing file is not a capture, and a zero-byte artefact
#   reads as favourable — measured three times in this tree.
if [ "$LINES" -lt 100 ]; then
  echo "★★★ THE GUEST CAPTURE IS EMPTY OR TINY ($LINES lines). Do NOT read this as"
  echo "    'the guest issued no ioctls' — it is far more likely the shim did not attach"
  echo "    or the pull failed. Every number derived from it below is VOID."
else
  echo "    ★ capture looks real: $LINES records, and it contains RM_CONTROL:"
  grep -c '"nr":42' "${OUTBASE}_r1.jsonl"
fi
echo "=== NVDIFF_HOOK_DONE ==="
