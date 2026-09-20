#!/usr/bin/env bash
# ★★★★★ THE RAW CLIENT ON BARE METAL — the baseline every guest verdict is read against.
#
# > Owner, 2026-09-20: *"The raw client has been the most valuable thing though… a full good
# > non racy pass on host is necessity to test, iterate and trust result for the guest."*
#
# ⊘ NO QEMU, NO KVM, NO GUEST. The same binary the guest runs, against the real driver on the
# real card. That is the whole point: an arm that fails HERE is a CLIENT or ENVIRONMENT fact,
# and an arm that passes here and fails in the guest **indicts kayfabe** — the owner's standing
# rule (`bare_metal_pass_guest_fail_indicts_kayfabe`, 2026-09-08). Without this baseline
# *"the client is probably fine"* is a belief, not a measurement.
#
# ⚠ Serialized against the guest lane on the SAME lock: both drive one GPU, and the fast lane's
# `pkill` would not save us here — bare metal has no QEMU to kill, it would simply contend.
set -uo pipefail
BENCH=${BENCH_DIR:-/workspace/bench}
TAG=${1:-bare}
BUDGET=${2:-120}
shift 2 2>/dev/null || true
BIN=${KF_LADDER:-/root/kayfabe/target/release/kayfabe-rm-ladder}
[ -x "$BIN" ] || { echo "bare_metal_suite: missing $BIN — cargo build --release -p kayfabe-isolate-host --bin kayfabe-rm-ladder"; exit 2; }

ARMS=("$@")
if [ "${#ARMS[@]}" -eq 0 ]; then
    ARMS=(--timer --engines --doorbell-census --gpu-info-sweep --bus-info-sweep --concurrency
          --defer-liveness --blockage-coverage --alias-two-vas --alias-unmap-observe
          --map-propagation --late-map-race --missing-page-fault --uvm-invalidate --uvm-mean
          --executor-vas --guest-ring-channel --dictated-ring --dictated-ring-negative
          --ce-client --ce-client-guest-ram --guest-ram-pin --bar1-crossing --atomics-probe
          --pce-mask-probe --gpga-reserve-probe --map-stress --concurrent-fuzz
          --cross-client-leak --rpc-mixed-allocs)
fi

exec 9>"${KF_LOCK:-/tmp/kayfabe-fastguest.lock}"
command -v flock >/dev/null 2>&1 && flock 9

# ★★★★★ THE CELL IDENTITY — this suite is an AXIS PROBE, not only a regression test.
#
# > `[owner, 2026-09-21]` *"it's also useful if the raw client 30/30 can be run on other GPUs and
# > drivers later, as it's a test for when iterating kayfabe itself on the axis matrix."*
#
# ★★★ Why it works as one, and it is a property no other lane has: **there is no VMM here.** The
# suite isolates the `(GPU die x host driver)` cell from everything kayfabe does, so a red arm on a
# new die is a fact about the DIE, never about our emulation. ⇒ It needs **no KVM box** — a plain
# CUDA container covers it, so this lane can fan out across many GPU types at container prices
# while the guest lane cannot.
#
# ★★★ And several arms are **already** per-die discriminators, not generic tests: `--doorbell-census`
# (⚠ `blackwell_doorbell_encoding_differs_per_die_group` — GB202 sets bit 30 where Ampere does not),
# `--pce-mask-probe`, `--gpga-reserve-probe`, `--atomics-probe`, `--bar1-crossing`. ⇒ On an unseen
# die those arms are a **difference DETECTOR**, and their failures are the exact input
# `DERIVE PER DIE, MAINTAIN PER FAMILY` asks for. A red arm here is a finding, not a defect.
#
# ⊘ Which is why the identity must be MACHINE-READABLE and on every run. A pass rate with no cell
# attached cannot be composed into a matrix, and `no provenance looks CLEANER than bad provenance`.
_q(){ nvidia-smi --query-gpu="$1" --format=csv,noheader 2>/dev/null | head -1 | sed 's/^ *//;s/ *$//'; }
GPU_NAME=$(_q name); GPU_DRV=$(_q driver_version); GPU_DEV=$(_q pci.device_id); GPU_CC=$(_q compute_cap)
GPU_UUID=$(_q uuid)
# ⊘ The open module and the proprietary one are DIFFERENT host drivers at the same version string;
# the axis is "host driver", and the module flavour is part of the cell.
KMOD=$(grep -q "Open Kernel Module" /proc/driver/nvidia/version 2>/dev/null && echo open || echo proprietary)
REV=$(cd "$(dirname "$0")/../.." 2>/dev/null && git rev-parse --short HEAD 2>/dev/null || echo '?')
echo "BARE_SUITE_STARTED=$(date -Is) arms=${#ARMS[@]} budget=${BUDGET}s"
echo "bin=$BIN  rev=$REV"
echo "gpu=$GPU_NAME  driver=$GPU_DRV($KMOD)  pci_dev=$GPU_DEV  cc=$GPU_CC"
# ⚠ Refuse rather than measure an unidentified cell: a row that cannot say WHICH die it describes
# poisons the matrix it lands in, and is worse than a missing row.
if [ -z "$GPU_NAME" ] || [ -z "$GPU_DRV" ]; then
    echo "bare_metal_suite: ⊘ REFUSING — cannot identify the GPU/driver cell (nvidia-smi gave nothing)."
    echo "  This is a HARNESS precondition, not a client result."
    exit 3
fi
# ⊘⊘⊘ THE LOG DIRECTORY IS A PRECONDITION, AND WITHOUT THIS CHECK ITS ABSENCE READS AS
# "the client failed 30 times". `[measured w822]` on a fresh box $BENCH did not exist, every
# arm's redirect failed, and the suite printed `client rc=1` for all 30 — attributing a HARNESS
# fault to the CLIENT. That is the misattribution this tree keeps paying for: an instrument that
# reports a number instead of refusing. A precondition is checked BEFORE the loop, by name.
mkdir -p "$BENCH" 2>/dev/null
if ! : > "$BENCH/.write_probe" 2>/dev/null; then
    echo "bare_metal_suite: ⊘ REFUSING — cannot write logs to $BENCH."
    echo "  This is a HARNESS precondition, not a client result. Set BENCH= to a writable dir."
    exit 3
fi
rm -f "$BENCH/.write_probe"
printf '%-28s %-9s %5s  %s\n' ARM VERDICT secs why
pass=0; fail=0; crash=0
for arm in "${ARMS[@]}"; do
    name=${arm#--}
    log=$BENCH/${TAG}_${name}.log
    s=$(date +%s)
    timeout --kill-after=5 "$BUDGET" "$BIN" "$arm" ${KF_LADDER_ARGS:-} > "$log" 2>&1
    rc=$?
    secs=$(( $(date +%s) - s ))
    # ⊘ The CLIENT'S own rc is the verdict, exactly as in the guest lane — a run that printed
    # something reassuring and exited non-zero is a failure, and the tail is not the verdict.
    if [ "$rc" = 124 ] || [ "$rc" = 137 ]; then
        v=CRASH; why="budget ${BUDGET}s exceeded"; crash=$((crash+1))
    elif [ "$rc" != 0 ]; then
        v=FAIL; why="client rc=$rc"; fail=$((fail+1))
    else
        v=PASS; why="${secs}s"; pass=$((pass+1))
    fi
    printf '%-28s %-9s %5s  %s\n' "$arm" "$v" "${secs}s" "$why"
    # ⊘ One machine-readable row PER ARM, carrying the cell. Collecting only the totals loses
    # exactly what the matrix is for: WHICH arm differs on WHICH die.
    echo "BARE_CELL_ARM arm=$name verdict=$v secs=$secs rc=$rc pci_dev=$GPU_DEV drv=$GPU_DRV kmod=$KMOD cc=$GPU_CC rev=$REV"
done
echo "BARE_SUITE_PASS=$pass BARE_SUITE_FAIL=$fail BARE_SUITE_CRASH=$crash ARMS=${#ARMS[@]}"
echo "BARE_CELL pci_dev=$GPU_DEV gpu=\"$GPU_NAME\" drv=$GPU_DRV kmod=$KMOD cc=$GPU_CC rev=$REV pass=$pass fail=$fail crash=$crash arms=${#ARMS[@]} uuid=$GPU_UUID"
