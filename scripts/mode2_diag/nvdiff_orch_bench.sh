#!/usr/bin/env bash
# nvdiff_orch_bench.sh — runs ON THE KAYFABE BENCH HOST (vh/vh2). Pushes the nvdiff
# instrument into an ALREADY-BOOTED Mode-2 guest, runs the capture, pulls it back.
#
# ⊘ Why this exists rather than the brief's `ssh vg 'bash -s' < nvdiff_run_guest.sh`:
# the `vg` ssh alias reaches the guest through `ProxyJump vh` on hostfwd port 2223, which
# is the C artifact's bench topology. The kayfabe bench has **no hostfwd at all** — the
# guest is on a tap at 192.168.77.2 and the only way in is `/workspace/bench/gssh_nv`.
# (CLAUDE.md already records the corrected form of this trap for the ssh-config half.)
#
#   stage the instrument onto the bench host first:
#     scp tests/mode2/nvdiff/{nvdiff_shim.c,nvd_prog.c,nvd_capture.sh,uvm_sizes.h} \
#         scripts/mode2_diag/nvdiff_run_guest.sh   vh2:/workspace/nvdiff_stage/
#   then:
#     ssh vh2 'NVD_STAGE=ctx NVD_RUNS=2 bash /workspace/nvdiff_stage/nvdiff_orch_bench.sh'
#
# ★ NVDIFF_MAXBUF is pinned to 65536 below. The host reference was captured there; the
# capture script's default of 8192 truncates 27 records and every one of them would read
# as a divergence that is a property of the instrument.
set -u
S=${NVD_STAGEDIR:-/workspace/nvdiff_stage}
G=${GSSH:-/workspace/bench/gssh_nv}
STAGE=${NVD_STAGE:-ctx}
RUNS=${NVD_RUNS:-2}

echo "=== push instrument into the guest ==="
for f in nvdiff_shim.c nvd_prog.c nvd_capture.sh uvm_sizes.h; do
    $G "cat > /tmp/$f" < "$S/$f" || { echo "PUSH FAIL $f"; exit 2; }
done
$G 'chmod +x /tmp/nvd_capture.sh; md5sum /tmp/nvdiff_shim.c /tmp/nvd_prog.c /tmp/uvm_sizes.h'

echo "=== run the guest half (stage=$STAGE runs=$RUNS) ==="
$G "NVD_STAGE=$STAGE NVD_RUNS=$RUNS NVDIFF_MAXBUF=65536 bash -s" < "$S/nvdiff_run_guest.sh"
RC=$?
echo "=== guest runner rc=$RC ==="

echo "=== pull captures back to the bench host ==="
mkdir -p /workspace/bench/nvdiff_guest
$G "cd /tmp/nvd_guest && tar cf - ${STAGE}_r*.jsonl ${STAGE}_r*.stdout env_${STAGE}.txt ${STAGE}_dmesg.log 2>/dev/null" \
    | tar xf - -C /workspace/bench/nvdiff_guest
ls -l /workspace/bench/nvdiff_guest
exit $RC
