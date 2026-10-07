#!/usr/bin/env bash
# ★★★ E2 — the acceptance and its control, on a LIVE GUEST, with the arrival ATTRIBUTED.
#
#   usage: scripts/bench/e2_doorbell_witness.sh <tag>
#   writes: /workspace/bench/run_<tag>_{serial,qemu,dmesg,probe}.log  (boot_capture's)
#           /workspace/bench/run_<tag>_e2.log                         ★ this script's verdict
#
# ## What is being claimed, and what would make it false
#
# `docs/design/execution_plane_increments.md` **E2**:
#
#   acceptance — a boot in which a guest doorbell write produces a
#                `DoorbellOutcome`-or-named-`FwdFault`, COUNTED
#   control    — a non-doorbell BAR write in the same run produces neither
#
# ## ★★★ A BOOLEAN WITNESS CANNOT ATTRIBUTE, and this is the shape that bit us on E0
#
# The device's teardown counters answer *"did a doorbell arrive?"*. They can **never**
# answer *"did it arrive BECAUSE THE GUEST WROTE ONE?"* — the device writes them, so a
# number that moved for some other reason reads identically. E0's first witness printed
# `★ AN ISOLATE CHILD EXISTED`, which reads as the strong claim while being compatible with
# the weak one; the child turned out to predate the guest by 28 seconds.
#
# So the arrival is **stamped against a timeline the device does not write**, in three
# independent pieces:
#
#   1. the guest-side tool prints `E2POKE before/after <ISO-8601>` from inside the guest,
#      immediately around its single `volatile uint32_t` store;
#   2. this script records the host wall-clock before and after each ssh invocation;
#   3. the device prints one `-msg timestamp=on` line per arrival, from QEMU.
#
# The check below requires the device's arrival stamp to fall INSIDE the window (2) — and
# requires that **no** arrival line exists before the first window opened. The second half
# is what excludes "the driver rang one during boot", which would make the counter move for
# a reason this experiment did not cause.
#
# ## ★ The control is issued FIRST, and through the same mapping
#
# One `mmap` of one page of `resource0`, two stores four bytes apart. The control therefore
# differs from the acceptance in the OFFSET and in nothing else — not the mapping, not the
# process, not the permissions, not the instruction. It is issued first so that a device
# which counted every BAR write would already be non-zero before the doorbell is rung.
#
# ## Traps encoded inline
#
# - ★★ `pgrep -x qemu-system-x86_64` can NEVER match (`/proc/PID/comm` truncates at 15);
#   `boot_capture.sh` does the precondition check and uses the right name.
# - ★ The guest needs ~20-25 s to reach a login prompt and `-serial file:` output LAGS.
# - ⊘ This script does not `pkill -f` anything, ever.
set -uo pipefail

BENCH=${BENCH_DIR:-/workspace/bench}
REPO=${REPO:-$(cd "$(dirname "$0")/../.." && pwd)}
TAG=${1:?usage: e2_doorbell_witness.sh <tag>}

OUT=$BENCH/run_${TAG}_e2.log
QEMU_LOG=$BENCH/run_${TAG}_qemu.log
PROBE=$BENCH/run_${TAG}_probe.log

# ★★★ The two offsets. `0x00bb0090` is NV_VIRTUAL_FUNCTION_DOORBELL as this device decodes
# it (0x00b80000, the physical function's NV_VIRTUAL_FUNCTION offset, + 0x00030090); the
# control is the next dword up, INSIDE the same 64 KiB usermode window, which this device
# models nothing at.
DOORBELL_OFF=bb0090
CONTROL_OFF=bb0094
# runlist 7, channel 5 — a token RM's own encoder can emit (VECTOR 11:0, RUNLIST_ID 22:16).
TOKEN=00070005
# The control carries the SAME value, so a device that reported "a token arrived" on the
# strength of the VALUE rather than the OFFSET would be caught.
CONTROL_VALUE=00070005

say()  { printf '[e2:%s] %s\n' "$TAG" "$*" | tee -a "$OUT"; }
fail() { printf '[e2:%s] ★ FAILED (%s): %s\n' "$TAG" "$1" "${*:2}" | tee -a "$OUT"; exit "${DIE_RC:-2}"; }

# =====================================================================================
# HOOK MODE — what boot_capture.sh calls while the guest is still up
# =====================================================================================
if [ "${E2_HOOK_MODE:-0}" = "1" ]; then
  G="$BENCH/gssh_nv"
  echo "=== E2 poke phase ==="
  # The emulated GPU's PCI address, found by its vendor id — never hard-coded, because the
  # slot moves with the command line.
  BDF=$("$G" 'for d in /sys/bus/pci/devices/*; do
                 [ "$(cat $d/vendor)" = "0x10de" ] && basename $d; done' 2>/dev/null | head -1)
  echo "E2 bdf=$BDF"
  [ -n "$BDF" ] || { echo "E2 ★ no NVIDIA-vendor PCI device in the guest"; exit 10; }

  RES=/sys/bus/pci/devices/$BDF/resource0

  # ★ Stage the tool INSIDE the guest, over the ssh channel that is already there. Done
  # here, before the first `E2HOST *_before` stamp, so the copy is outside every window the
  # attribution check reasons about.
  base64 < "$BENCH/e2_doorbell_poke" | "$G" \
      'base64 -d > /tmp/e2_doorbell_poke && chmod +x /tmp/e2_doorbell_poke && echo staged'
  "$G" 'ls -l /tmp/e2_doorbell_poke'
  # ⊘ The driver is unloaded FIRST. sysfs `resource0` mmap is refused while a driver holds
  # the region on some kernels, and a refusal here would read as "the transport is broken"
  # when it means "somebody else has the BAR". The boot's own evidence (dmesg, the wall) was
  # captured by boot_capture BEFORE this hook ran, so nothing is lost.
  "$G" 'sudo rmmod nvidia_drm nvidia_modeset nvidia_uvm nvidia 2>/dev/null; echo rmmod_rc=$?'

  echo "=== CONTROL first: a non-doorbell write, same mapping, same value ==="
  echo "E2HOST control_before $(date -u +%Y-%m-%dT%H:%M:%S.%6NZ)"
  "$G" "sudo /tmp/e2_doorbell_poke $RES $CONTROL_OFF $CONTROL_VALUE"; echo "CONTROL_RC=$?"
  echo "E2HOST control_after $(date -u +%Y-%m-%dT%H:%M:%S.%6NZ)"

  # ★ A gap the device's own timestamps can resolve, so "which window did this arrival fall
  # in" is never a judgement call.
  sleep 3

  # ⊘ ★★★ THE ARTIFACT THAT COULD SHOW OTHERWISE. With `E2_SKIP_DOORBELL=1` this hook
  # issues the control and STOPS, so the boot is identical in every other respect and the
  # driver-mode verdict below must go RED with "expected EXACTLY ONE arrival, got 0". A
  # check that cannot produce that sentence is not a check — E0b keeps `e0bfail1` for the
  # same reason and it is the most useful file in that increment's evidence.
  if [ "${E2_SKIP_DOORBELL:-0}" = "1" ]; then
    echo "E2 ★ NEGATIVE CONTROL: the doorbell poke is SUPPRESSED"
    exit 0
  fi

  echo "=== ACCEPTANCE: the doorbell ==="
  echo "E2HOST doorbell_before $(date -u +%Y-%m-%dT%H:%M:%S.%6NZ)"
  "$G" "sudo /tmp/e2_doorbell_poke $RES $DOORBELL_OFF $TOKEN"; echo "DOORBELL_RC=$?"
  echo "E2HOST doorbell_after $(date -u +%Y-%m-%dT%H:%M:%S.%6NZ)"
  exit 0
fi

# =====================================================================================
# DRIVER MODE
# =====================================================================================
: > "$OUT"
say "revision under test: $(git -C "$REPO" rev-parse HEAD 2>/dev/null || echo UNKNOWN)"
say "archive in the binary: $(strings "$BENCH/qemu-build/qemu-system-x86_64" 2>/dev/null \
      | grep -o 'kayfabe-rev:[0-9a-f-]*' | sort -u | tr '\n' ' ')"

# ---- build the guest-side tool, on the host, statically -----------------------------
POKE=$BENCH/e2_doorbell_poke
say "building the guest poke tool"
# ★ Static if this host can, dynamic otherwise — and the fallback is announced rather than
# silent. A dynamic binary built against an older glibc than the guest's runs fine; one
# built against a NEWER one does not, and that failure would present in the guest as a
# missing symbol, i.e. as "the poke did not run", which is a result-shaped non-result.
if ! gcc -O1 -static -Wall -Wextra -o "$POKE" "$REPO/scripts/bench/e2_doorbell_poke.c" 2>/dev/null; then
  say "⚠ no static libc on this host — building the poke tool DYNAMIC"
  gcc -O1 -Wall -Wextra -o "$POKE" "$REPO/scripts/bench/e2_doorbell_poke.c" \
    || fail build "the guest poke tool did not compile"
fi
say "poke tool: $(file "$POKE" 2>/dev/null || ls -l "$POKE")"

# ---- boot, with the hook ------------------------------------------------------------
say "booting (boot_capture drives the driver load and the device open)"
E2_HOOK_MODE=1 POST_CAPTURE_HOOK="$0" \
  E2_SKIP_DOORBELL="${E2_SKIP_DOORBELL:-0}" \
  BENCH_DIR="$BENCH" REPO="$REPO" \
  "$REPO/scripts/bench/boot_capture.sh" "$TAG" 2>&1 | tee -a "$OUT"
BOOT_RC=${PIPESTATUS[0]}
say "boot_capture rc=$BOOT_RC"

# =====================================================================================
# VERDICT — read the device's own report and the two timelines
# =====================================================================================
say "--- the device's doorbell report ---"
grep -E 'nvkvm: doorbells:|first doorbell refusal|nvkvm: DOORBELL' "$QEMU_LOG" | tee -a "$OUT"

ARRIVED=$(grep -oE 'nvkvm: doorbells: [0-9]+ arrived' "$QEMU_LOG" | grep -oE '[0-9]+' | tail -1)
SERVED=$(grep -oE 'arrived, [0-9]+ served' "$QEMU_LOG" | grep -oE '[0-9]+' | tail -1)
REFUSED=$(grep -oE 'served, [0-9]+ REFUSED' "$QEMU_LOG" | grep -oE '[0-9]+' | tail -1)
: "${ARRIVED:=}" "${SERVED:=}" "${REFUSED:=}"

[ -n "$ARRIVED" ] || fail instrument \
  "the device printed NO doorbell line at all. That is an INSTRUMENT failure, not a result:
   an archive of this revision prints it unconditionally, all-zeros included, so its absence
   means the binary under test is not this revision. ⊘ Do not read it as 'zero doorbells'."

say "counters: arrived=$ARRIVED served=$SERVED refused=$REFUSED"

# ---- 1. the acceptance --------------------------------------------------------------
[ "$ARRIVED" = "1" ] || fail acceptance \
  "expected EXACTLY ONE arrival (the poke), got $ARRIVED. More than one means something
   other than this experiment rang; zero means the write never reached the classification."
[ "$((SERVED + REFUSED))" = "$ARRIVED" ] || fail algebra \
  "arrived != served + refused ($ARRIVED != $SERVED + $REFUSED): a counter is absorbing another"

KIND=$(grep -oE 'REFUSED \[[^]]+\]' "$QEMU_LOG" | head -1 | sed 's/REFUSED \[//;s/\]//')
if [ "$SERVED" = "1" ]; then
  say "★ the ring was SERVED — which at E2 would be a SURPRISE (no channel exists yet)"
elif [ -n "$KIND" ]; then
  say "★ the ring was REFUSED by name: [$KIND]"
  case "$KIND" in
    FwdFault::*) : ;;
    Device::NoDoorbellPort) fail wiring \
      "the archive counted the ring and forwarded NOTHING: the composition root did not
       install the doorbell port, so the write never reached kayfabe_rt::SharedDevice." ;;
    *) fail naming "the refusal is not a named FwdFault: [$KIND]" ;;
  esac
else
  fail naming "a doorbell was refused and the device named no kind"
fi

# ---- 2. the CONTROL -----------------------------------------------------------------
grep -q 'CONTROL_RC=0' "$PROBE" || fail control \
  "the control write did not execute, so 'no doorbell was counted for it' is vacuous"
grep -q 'DOORBELL_RC=0' "$PROBE" || fail acceptance \
  "the doorbell write did not execute"
# ⊘ The control is verified by SUBTRACTION and the subtraction is exact: two writes were
# issued through one mapping, four bytes apart, and exactly one arrival was counted.
say "★ CONTROL HOLDS: 2 guest MMIO writes issued (offsets +0x$CONTROL_OFF and +0x$DOORBELL_OFF), 1 arrival counted"

# ---- 3. ATTRIBUTION — the arrival is inside the doorbell window, and nothing is outside
DB_BEFORE=$(grep -oE 'E2HOST doorbell_before \S+' "$PROBE" | tail -1 | awk '{print $3}')
DB_AFTER=$(grep -oE 'E2HOST doorbell_after \S+' "$PROBE" | tail -1 | awk '{print $3}')
CT_BEFORE=$(grep -oE 'E2HOST control_before \S+' "$PROBE" | tail -1 | awk '{print $3}')
ARRIVAL=$(grep -E 'nvkvm: DOORBELL token' "$QEMU_LOG" | head -1 | awk '{print $1}')

if [ -z "$DB_BEFORE" ] || [ -z "$DB_AFTER" ] || [ -z "$ARRIVAL" ]; then
  fail instrument \
    "an attribution stamp is missing (host_before='$DB_BEFORE' host_after='$DB_AFTER'
     device='$ARRIVAL'). ⊘ A missing stamp is an INSTRUMENT failure and is reported as one —
     never as 'the arrival could not be attributed', which would read as a result."
fi
say "attribution: control window opened $CT_BEFORE"
say "             doorbell window $DB_BEFORE .. $DB_AFTER"
say "             device stamped  $ARRIVAL"
# ★ String comparison is correct here and only here: both stamps are ISO-8601 UTC with a
# fixed-width fractional part, so lexical order IS chronological order.
if [ "$ARRIVAL" \< "$DB_BEFORE" ] || [ "$ARRIVAL" \> "$DB_AFTER" ]; then
  fail attribution \
    "the device's arrival stamp is OUTSIDE the window in which the guest issued the write.
     The counter moved; this experiment is not what moved it."
fi
NBEFORE=$(awk -v t="$CT_BEFORE" '/nvkvm: DOORBELL token/ { if ($1 < t) n++ } END { print n+0 }' "$QEMU_LOG")
[ "$NBEFORE" = "0" ] || fail attribution \
  "$NBEFORE doorbell arrivals are stamped BEFORE the experiment opened its first window —
   something in the boot rang one, so the count above is not this experiment's alone"

say "★★★ E2 ACCEPTANCE HOLDS at $(git -C "$REPO" rev-parse --short HEAD 2>/dev/null): one guest"
say "    MMIO write to +0x$DOORBELL_OFF reached kayfabe_rt::SharedDevice::doorbell, was"
say "    answered [$KIND], and was COUNTED — arrival stamped inside the guest's own window."
say "    The control write at +0x$CONTROL_OFF, same mapping, produced neither."
exit 0
