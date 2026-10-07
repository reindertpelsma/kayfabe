#!/usr/bin/env bash
# ★★★★★ w315 — ATTRIBUTE THE ~100 ms PER-LAUNCH FLOOR. One boot per arm.
#
#   usage: scripts/bench/w315_floor.sh <arm>
#     base    — the instrument OFF. THE BASELINE, and the thing every other arm is read
#               against. An un-armed boot is byte-for-byte master's.
#     census  — the aggregate breakdown only. ⊘ No per-event lines, so the print cost is
#               ~4 lines per boot rather than ~900: this is the arm whose NUMBERS are read.
#     full    — per-event lines as well. Read for ALIGNMENT (which doorbell fell inside
#               which launch) and to price the instrument's own printing.
#     inject  — ★ THE KNOWN-POSITIVE. A known delay into a NAMED segment. The census MUST
#               show it landing there, moving no other segment, and the GUEST must see it.
#
# ## ⊘⊘ WHAT THIS RUNG MAY NOT DO, and why the arms are shaped this way
#
# w311 measured the floor and nearly shipped the wrong MECHANISM for it: the device's
# `SEMA-WRITE` lines arrive on a hard 251 ms cadence and `251/2 = 125.5 ms` matched the
# fitted fixed cost `C ≈ 115–132 ms` to 0.4 %. It arrived PRE-CORROBORATED and was refuted
# only by the guest's own latency distribution. The 251 ms was `OBSERVER_TICK_MS = 250` —
# the observer thread's epoll timeout, i.e. the instrument's clock impersonating the
# measured quantity.
#
# ⇒ **A cadence that matches your quantity is a SUSPECT, not a corroboration.** Nothing here
#   infers a mechanism from a fit, a period, or a coincidence of magnitudes. Every number is
#   a bracketed interval, and the `inject` arm exists so the bracketing itself is falsifiable.
#
# ## ★★★ PRE-REGISTERED, BEFORE THE FIRST BOOT — five outcomes, so none reads as favourable
#
#   (A) ONE SEGMENT DOMINATES        ⇒ name it, with the number. That is the fix target.
#   (B) IT IS SPREAD ACROSS MANY     ⇒ ★ ALSO A FULL RESULT, and a far more expensive one to
#                                      act on. Say so plainly; do NOT pick the largest and
#                                      call it the cause.
#   (C) THE FLOOR DOES NOT REPRODUCE ⇒ suspect the instrument AND suspect that observation
#       UNDER INSTRUMENTATION          perturbs it. Report BOTH readings (`base` vs `census`).
#   (D) THE BREAKDOWN DOES NOT SUM   ⇒ ★★★ THE MISSING TIME IS THE FINDING. It must be NAMED
#                                      (`UNMARKED_ms`, and the guest-minus-host residual),
#                                      never distributed across the segments that did report.
#   (E) NO TRUSTWORTHY CLOCK         ⇒ report guest-side segmentation only, EXPLICITLY
#       CORRESPONDENCE                 unattributed across the boundary.
#
# ## ⚠ The clock correspondence, stated once
#
# Host segments are `Instant` on the host's CLOCK_MONOTONIC, taken on the vCPU thread.
# The guest measures with the GUEST's CLOCK_MONOTONIC. **No offset between them is computed
# anywhere.** The only correspondence claimed is NESTING: a guest MMIO write is a vmexit, so
# the guest is halted for the whole trap, and every host interval reported here is therefore
# contained inside the guest's launch window. That licenses `Σ host ≤ launch_ms` and nothing
# else. Rate agreement is checked separately, from the two runs' own spans.
set -uo pipefail
ARM="${1:-}"
case "$ARM" in base|census|full|inject) ;;
  *) echo "usage: $0 base|census|full|inject" >&2; exit 64 ;;
esac

REPO=${KAYFABE_REPO:-/workspace/kayfabe_w315}
export KAYFABE_REPO="$REPO"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w315}
export KAYFABE_TAG=${KAYFABE_TAG:-w315$ARM}
export POST_CAPTURE_HOOK="$REPO/scripts/bench/cup8bench_hook.sh"
export GQ_TIMEOUT=${GQ_TIMEOUT:-900}

# ★ ONE SIZE, and it is the SMALL one. The floor is a FIXED cost: at N=512 it is ~114 of the
#   126 ms (w311 §2.1), at N=2048 it is ~455 of 624. The small size is where the fixed term
#   is the largest FRACTION of the measurement, so it is where an attribution is cleanest —
#   and it is the fastest, which is what makes four boots affordable.
# ⊘ Sizes/iters are overridable but DEFAULTED, so all four arms are one-variable by default.
export KAYFABE_BENCH_SIZES=${KAYFABE_BENCH_SIZES:-512}
export KAYFABE_BENCH_ITERS=${KAYFABE_BENCH_ITERS:-12}
export KAYFABE_BENCH_BATCH=${KAYFABE_BENCH_BATCH:-10}
export KAYFABE_BENCH_VERIFY=${KAYFABE_BENCH_VERIFY:-1}
# ⊘ `measure` only. The negative control is a SECOND CUDA context and hangs at `cuCtxCreate`
#   behind the first process's teardown (known #12, w311 §8), costing a boot to learn nothing
#   this rung is about. ⇒ the `bad=0` these runs report is UNGUARDED IN THIS RUNG and every
#   report of it must say so. Attribution, not correctness, is the deliverable.
export KAYFABE_BENCH_ONLY=${KAYFABE_BENCH_ONLY:-measure}

# ---- THE ONE VARIABLE ------------------------------------------------------------------
# ⊘ Every arm runs the SAME BINARY. `kftime` is chosen by environment, never by feature, so
#   an evidence run and its control cannot differ in anything but this variable — the same
#   split `KAYFABE_ISOLATES` already uses, and for the same reason.
case "$ARM" in
  base)   unset KAYFABE_KFTIME KAYFABE_KFTIME_INJECT_US KAYFABE_KFTIME_INJECT_SEG ;;
  census) export KAYFABE_KFTIME=census ;;
  full)   export KAYFABE_KFTIME=on ;;
  inject) export KAYFABE_KFTIME=census
          # 30 ms — chosen to be LARGE against the per-segment means (tens to hundreds of µs)
          # and SMALL against the ~100 ms floor, so it is unambiguous in the census AND
          # visible in the guest's own latency without swamping it.
          export KAYFABE_KFTIME_INJECT_US=${KAYFABE_KFTIME_INJECT_US:-30000}
          export KAYFABE_KFTIME_INJECT_SEG=${KAYFABE_KFTIME_INJECT_SEG:-vas_publish} ;;
esac
export KAYFABE_KFTIME_CENSUS_EVERY=${KAYFABE_KFTIME_CENSUS_EVERY:-200}

# ★★★★★ **THE VMEXIT SAMPLER — and it is the single most valuable number this rung can get.**
#
# `[measured 2026-08-14]` **THIS BENCH IS ITSELF A KVM GUEST** (`systemd-detect-virt` → `kvm`,
# `hypervisor` in `/proc/cpuinfo`, nested KVM present, Xeon W-2133). ⇒ our guest runs at **L2**
# and every MMIO access takes a NESTED vmexit (L2 → L1 → L0). The C artifact attributes a 2.5×
# throughput gap to exactly this: llama.cpp at **49.9 tok/s bare metal vs ~20 on a nested vast
# box**, *"entirely nested-virt vmexit tax, not Mode-2 design"*
# (`C: /workspace/nvidia-gpu-passthrough/docs/MILESTONES.md:12-14`).
#
# ⊘ The vmexit is ALREADY OVER when any `kftime` hook runs, so trap-shaped cost cannot appear
# as a segment — it appears only as the analyser's `UNACCOUNTED` row. What bounds it is the
# **exit COUNT**, and that is what this samples. ★ A count settles in one number what a finer
# segment split cannot settle at all.
#
# ⚠ These are the counters of the KVM instance running OUR guest, on THIS box (L1). They are
# host-wide, so anything else using /dev/kvm here pollutes them — which is why the bench lock
# is held and `pgrep -x qemu-system-x86` is checked. ⊘ `qemu-system-x86_64` can never match:
# /proc/PID/comm truncates at 15 chars.
KVMDIR=/sys/kernel/debug/kvm
EXITLOG=/workspace/bench/run_${KAYFABE_TAG}_kvmexits.log
{
  echo "# w315 vmexit sampler tag=$KAYFABE_TAG start=$(date -Is)"
  echo "# epoch_ms exits mmio_exits io_exits irq_exits halt_exits qemu_running"
} > "$EXITLOG"
(
  while :; do
    R=$(pgrep -x qemu-system-x86 >/dev/null && echo 1 || echo 0)
    E=$(cat $KVMDIR/exits 2>/dev/null || echo NA)
    M=$(cat $KVMDIR/mmio_exits 2>/dev/null || echo NA)
    I=$(cat $KVMDIR/io_exits 2>/dev/null || echo NA)
    Q=$(cat $KVMDIR/irq_exits 2>/dev/null || echo NA)
    H=$(cat $KVMDIR/halt_exits 2>/dev/null || echo NA)
    echo "$(date +%s%3N) $E $M $I $Q $H $R"
    sleep 1
  done
) >> "$EXITLOG" 2>&1 &
SAMPLER=$!
# ⊘ A trap, not a trailing kill: if the boot dies the sampler must die with it, or the next
# arm inherits a writer into a log it is about to read. `143` and `124` mean opposite things
# and neither of them stops a detached loop.
trap 'kill $SAMPLER 2>/dev/null' EXIT INT TERM

rm -f /workspace/bench/qemu-build/qemu-system-x86_64
"$REPO/scripts/bench/w290p_run.sh" "${W315_ARM:-drain}"
BRC=$?
kill $SAMPLER 2>/dev/null; wait $SAMPLER 2>/dev/null
echo "# end=$(date -Is)" >> "$EXITLOG"

OUT=/workspace/${KAYFABE_TAG}.log
Q=/workspace/bench/run_${KAYFABE_TAG}_qemu.log
P=/workspace/bench/run_${KAYFABE_TAG}_probe.log
D=/workspace/bench/run_${KAYFABE_TAG}_hostdmesg.log

{
echo ""
echo "================================================================================"
echo "=== ★★★★★ W315 arm=$ARM  inner_rc=$BRC  $(date -Is)"
echo "================================================================================"
echo "    KAYFABE_KFTIME=[${KAYFABE_KFTIME:-<unset>}] INJECT_US=[${KAYFABE_KFTIME_INJECT_US:-0}]"
echo "    INJECT_SEG=[${KAYFABE_KFTIME_INJECT_SEG:-<none>}] CENSUS_EVERY=[$KAYFABE_KFTIME_CENSUS_EVERY]"
echo "    ⊘ THE ARMING AS THE DEVICE SAW IT (a script exporting a variable is not a device reading one):"
grep -m1 'KFTIME ARMED' "$Q" 2>/dev/null | sed 's/^/      /' || true
if [ "$ARM" = base ]; then
  N=$(grep -c 'KFTIME' "$Q" 2>/dev/null)
  echo "      base arm: KFTIME lines in the device log = [$N] — MUST be 0, or this is not a baseline"
fi

echo ""
echo "=== ★★★ THE GUEST'S OWN SEGMENTATION — submit vs completion, its own CLOCK_MONOTONIC"
echo "    ⊘ This half needs no clock correspondence at all: both halves are the guest's."
grep -h '^GUEST_BSUM ' "$P" 2>/dev/null | sed 's/^/    /'
echo "    per-iteration (launch = submit + sync, by construction):"
grep -h '    ITER N=' "$P" 2>/dev/null | tail -20 | sed 's/^/    /'

echo ""
echo "=== ★★★★★ THE HOST-SIDE BREAKDOWN — the deliverable"
echo "    ⊘ Read UNMARKED_ms FIRST. A breakdown that does not sum to its bracket means the"
echo "      missing time is real and must be NAMED, not spread over the segments that did report."
awk '/KFTIME-CENSUS/{p=1} p&&/KFTIME-(CENSUS|SEG|NESTED|HIST)/{print "    "$0} /KFTIME-HIST/{p=0}' "$Q" 2>/dev/null | tail -80

echo ""
echo "=== ★ THE DOORBELL POPULATION — a per-launch cost needs a per-launch COUNT"
for k in mmio_doorbell mmio_other doorbell_ce doorbell_fwd; do
  L=$(grep "KFTIME-CENSUS kind=$k " "$Q" 2>/dev/null | tail -1)
  echo "    ${L:-⊘ kind=$k NEVER RECORDED — the hook did not fire, which is a statement about the hook}"
done
echo "    doorbells (device counter) = [$(grep -oE 'doorbells [0-9]+ arrived' "$Q" 2>/dev/null | tail -1)]"
echo "    ★DRAINED rows              = [$(grep -c '★DRAINED' "$Q" 2>/dev/null)]"

if [ "$ARM" = inject ]; then
  echo ""
  echo "=== ★★★★★ THE KNOWN-POSITIVE — watched ATTRIBUTING, or it is not an instrument"
  echo "    injected ${KAYFABE_KFTIME_INJECT_US} us into segment [${KAYFABE_KFTIME_INJECT_SEG}]."
  echo "    ⊘ THREE things must ALL hold, and any one of them failing kills the reading:"
  echo "      1. that segment's mean_us grew by ~${KAYFABE_KFTIME_INJECT_US}"
  echo "      2. NO other segment moved"
  echo "      3. the GUEST's launch_ms grew — otherwise the segment is not on the guest's path"
  grep -E "KFTIME-SEG ${KAYFABE_KFTIME_INJECT_SEG} " "$Q" 2>/dev/null | tail -2 | sed 's/^/      /'
fi

echo ""
echo "=== ★★★★★ VMEXITS — the bench is NESTED, so a COUNT bounds what a segment cannot"
echo "    ⊘ This box is itself a KVM guest ⇒ our guest is L2 and every MMIO access is a"
echo "      NESTED vmexit. The C blames a 2.5x llama.cpp gap on exactly that. The vmexit is"
echo "      OVER before any kftime hook runs, so it can only show up as UNACCOUNTED."
if [ -s "$EXITLOG" ]; then
  echo "    sampler rows = [$(grep -c '^[0-9]' "$EXITLOG")]  (1 Hz; ⊘ host-wide KVM counters)"
  python3 - "$EXITLOG" <<'PYEXIT' || echo "    ⊘ the sampler reducer refused — UNMEASURED"
import sys
rows=[l.split() for l in open(sys.argv[1]) if l[:1].isdigit()]
rows=[r for r in rows if len(r)>=7 and r[1]!="NA"]
if len(rows)<2:
    print("    ⊘ NO USABLE SAMPLER ROWS — UNMEASURED, not zero"); raise SystemExit(0)
# ⊘⊘ **LAST MINUS FIRST IS WRONG HERE, AND IT READ AS `exits=0`.** `/sys/kernel/debug/kvm/*`
# is per-LIVE-VM: the counters are 0 before QEMU starts and 0 again after it exits, so the
# obvious delta is exactly zero on a completed boot — a number that looks like a measurement
# and is an artefact of the VM being gone. ⇒ sum the POSITIVE per-row deltas instead, which
# also drops the single negative row where the VM disappeared.
tot={k:0 for k in range(1,6)}
peak=0; peak_t=0; span=0.0
for a,b in zip(rows,rows[1:]):
    dt=(int(b[0])-int(a[0]))/1000.0
    d={k:int(b[k])-int(a[k]) for k in range(1,6)}
    if d[1]<0:            # the VM went away between these two samples
        continue
    if any(v!=0 for v in d.values()):
        span+=dt
    for k in range(1,6): tot[k]+=d[k]
    if dt>0 and d[2]/dt>peak: peak,peak_t=d[2]/dt,(int(a[0])-int(rows[0][0]))/1000.0
print(f"    whole-VM totals (Σ positive per-row deltas; ⊘ NOT last-minus-first — the counters")
print(f"    reset to 0 when the VM exits, so that difference is 0 and means nothing):")
print(f"      exits={tot[1]} mmio_exits={tot[2]} io_exits={tot[3]} irq_exits={tot[4]} halt_exits={tot[5]}")
print(f"      active wall={span:.1f}s  ⇒ {tot[1]/span if span else 0:.0f} exits/s, {tot[2]/span if span else 0:.0f} mmio_exits/s")
print(f"    ★ PEAK mmio_exits/s = {peak:.0f} at t=+{peak_t:.0f}s from sampler start")
print(f"      ⊘ A 1 Hz sampler cannot see inside a 112 ms launch: the peak is an average over")
print(f"        a second that also contains the memset, the readback and the verify.")
PYEXIT
else
  echo "    ⊘ THE SAMPLER LOG IS EMPTY OR ABSENT — UNMEASURED. An empty artefact reads as"
  echo "      benign; this line exists so it does not."
fi
echo "    our device's own MMIO census (every one of these was a vmexit):"
for k in mmio_read mmio_doorbell mmio_other; do
  echo "      $(grep "KFTIME-CENSUS kind=$k " "$Q" 2>/dev/null | tail -1 | grep -oE 'events=[0-9]+ total_ms=[0-9.]+ mean_us=[0-9]+' || echo "kind=$k ⊘ UNRECORDED")"
done

echo ""
echo "=== (E) REGRESSION CHECK — the NEW criterion; host_rows is printed, never graded"
"$REPO/scripts/bench/regression_check_e.sh" "$Q" "$D" 2>&1 | sed 's/^/    /'
echo "    (E) exit status = $?"

echo ""
echo "=== ⚠ XID BY IDENTITY (a count cannot see a substitution)"
grep -oE 'Xid [^,]*' "$D" 2>/dev/null | sort | uniq -c | sed 's/^/    /' || echo "    (none / empty delta)"

echo "=== W315 EXIT arm=$ARM rc=$BRC $(date -Is) ==="
} >> "$OUT" 2>&1

exit "$BRC"
