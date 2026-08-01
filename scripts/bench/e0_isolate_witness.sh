#!/usr/bin/env bash
# ★★★ E0's witness: does a GUEST device-path action cause a REAL host RM verb?
#
#   usage: scripts/bench/e0_isolate_witness.sh <tag> <stillborn|loopback|real|unset>
#
# Runs `scripts/bench/boot_capture.sh <tag>` with `KAYFABE_ISOLATES` set to the named plane
# (or left unset), while SAMPLING THE HOST for the sandboxed isolate child the composition
# root would spawn. Writes `/workspace/bench/run_<tag>_isolate.log` and prints a verdict.
#
# `docs/design/execution_plane_increments.md` §3.2 is the claim this script tests:
#
#   The guest's `GSP_RM_ALLOC` → `ObjectPolicy` → `Gpu::apply` → `IsolateFactory::spawn` →
#   a `clone`d, sandboxed child → `RmConnection::open`, which is rungs R0–R6b: real
#   `NV_ESC_REGISTER_FD`, `NV01_ROOT_CLIENT`, `NV01_DEVICE_0`, `NV20_SUBDEVICE_0` ioctls on
#   the host's own `/dev/nvidiactl` and `/dev/nvidia0`.
#
# ★★ **Why a live child is a SUFFICIENT witness, and not merely suggestive.**
# `RmConnection::open` is a `?`-chain: any failed RM ioctl aborts it, the child's hello
# frame carries `Reply::Failed`, `build_isolate` returns `Err`, and the parent turns that
# into a STILLBORN isolate — i.e. the child is dead. There is no arrangement in which a
# `--rm real` child is alive and its RM allocations did not succeed. The fds and the
# `/dev/nvidia*` mapping below are recorded anyway, because a witness that rests on one
# citation is a witness nobody can re-check.
#
# ⊘ **What this does NOT witness**: any verb the *forwarding* plane issues. E0 wires the
# isolate plane; no `VerbPlan` is executed, no doorbell is rung, no pushbuffer runs. The
# RM verbs seen here are the isolate's own bring-up.
#
# ## Traps encoded inline
#
# - ★★ `pgrep -x qemu-system-x86_64` **can never match**: `/proc/PID/comm` truncates at 15
#   characters. This script never uses the long name. It also never uses `pgrep -f`, which
#   matches this script's own command line.
# - ★★ The isolate's own `comm` is `kayfabe-isolate` — **exactly 15 characters**, i.e. one
#   more would have truncated too. This script therefore does NOT depend on `comm` at all:
#   it scans `/proc/*/cmdline`, which is not truncated.
# - ★ The child is `clone`d into a new PID namespace. It is still visible in the host's
#   `/proc`; the namespace only changes what IT can see.
# - ★ The isolate is spawned mid-`nvidia-smi` and is reaped when the guest's proc is
#   reaped or QEMU exits, so a single post-hoc `ps` can miss it entirely. Hence a sampler
#   that runs for the whole boot rather than a `ps` at the end.
set -uo pipefail

BENCH=${BENCH_DIR:-/workspace/bench}
REPO=$(cd "$(dirname "$0")/../.." && pwd)
TAG=${1:?usage: e0_isolate_witness.sh <tag> <stillborn|loopback|real|unset>}
PLANE=${2:?usage: e0_isolate_witness.sh <tag> <stillborn|loopback|real|unset>}
OUT=$BENCH/run_${TAG}_isolate.log

say() { printf '[e0:%s] %s\n' "$TAG" "$*"; }

case "$PLANE" in
  stillborn|loopback|real) export KAYFABE_ISOLATES="$PLANE" ;;
  unset) unset KAYFABE_ISOLATES ;;
  *) echo "★ unknown plane '$PLANE'"; exit 2 ;;
esac

# ★★ REV_UNDER_TEST, stamped and asserted. A silent fetch behind a pipe has already
# attributed one whole suite result to the wrong revision.
REV=$(git -C "$REPO" rev-parse HEAD)
DIRTY=$(git -C "$REPO" status --porcelain | wc -l)

{
  echo "=== e0_isolate_witness tag=$TAG at $(date -Is) ==="
  echo "REV_UNDER_TEST=$REV"
  echo "TREE_DIRTY_FILES=$DIRTY"
  echo "KAYFABE_ISOLATES=${KAYFABE_ISOLATES:-<unset>}"
  echo "qemu binary: $(ls -l "$BENCH/qemu-build/qemu-system-x86_64" 2>/dev/null)"
  echo "archive rev embedded in the qemu binary:"
  strings "$BENCH/qemu-build/qemu-system-x86_64" 2>/dev/null \
    | grep -o 'kayfabe-rev:[0-9a-f]*' | sort -u | sed 's/^/  /'
  echo "host gpu: $(nvidia-smi --query-gpu=name,driver_version --format=csv,noheader 2>&1)"
  echo
} > "$OUT"

# ---- the sampler --------------------------------------------------------------------
# Dumps EVERY distinct isolate-child pid once, with the time it first appeared, and keeps a
# running sample count so a child that dies immediately is distinguishable from one that
# lives.
sampler() {
  local seen=0
  local -A latched=()
  while :; do
    for c in /proc/[0-9]*/cmdline; do
      p=${c%/cmdline}; p=${p#/proc/}
      # ★ cmdline is NUL-separated and, unlike `comm`, is NOT truncated. `2>/dev/null`
      # because a pid can exit between the glob and the read; that is not a finding.
      cl=$(tr '\0' ' ' < "$c" 2>/dev/null) || continue
      case "$cl" in
        *kayfabe-isolate*)
          seen=$((seen + 1))
          # ★★ EVERY distinct pid is dumped, not just the first. Which spawns happen and
          # WHEN is the whole question E0's evidence turns on: a child that appears before
          # the guest driver is even loaded was spawned by device REALIZE, and one that
          # appears during the device open was spawned by GUEST traffic. A sampler that
          # latched only the first sighting cannot tell those apart, and the first version
          # of this script could not.
          if [ -z "${latched[$p]:-}" ]; then
            latched[$p]=1
            {
              echo "=== ISOLATE CHILD pid $p  at $(date -Is)  (t+$(( $(date +%s) - T0 ))s from witness start) ==="
              echo "cmdline: $cl"
              echo "ppid   : $(awk '/^PPid:/{print $2}' /proc/$p/status 2>/dev/null)"
              echo "ppid is: $(tr '\0' ' ' < /proc/$(awk '/^PPid:/{print $2}' /proc/$p/status 2>/dev/null)/cmdline 2>/dev/null | cut -c1-100)"
              echo "--- open descriptors (the RM nodes are the point) ---"
              ls -l /proc/$p/fd 2>/dev/null | sed 's/^/  /'
              echo "--- mappings naming an nvidia node (an RM-SERVED mmap) ---"
              grep -i nvidia /proc/$p/maps 2>/dev/null | sed 's/^/  /' || echo "  (none)"
              echo "--- containment ---"
              grep -E '^(Name|Uid|Gid|NoNewPrivs|Seccomp|CapEff|CapPrm|CapBnd|Threads)' \
                   /proc/$p/status 2>/dev/null | sed 's/^/  /'
              echo "  namespaces (vs this shell's):"
              for ns in user pid net mnt ipc uts; do
                printf '    %-5s child=%s  host=%s\n' "$ns" \
                  "$(readlink /proc/$p/ns/$ns 2>/dev/null)" "$(readlink /proc/self/ns/$ns)"
              done
              echo
            } >> "$OUT"
          fi
          ;;
      esac
    done
    echo "$seen" > /tmp/e0_seen_$TAG
    sleep 0.5
  done
}

T0=$(date +%s)
export T0 OUT
echo 0 > /tmp/e0_seen_$TAG
sampler &
SPID=$!
trap 'kill -9 $SPID 2>/dev/null' EXIT

say "sampling for isolate children; plane=${KAYFABE_ISOLATES:-<unset>} rev=$REV"
# ★★ boot_capture's own phase lines are TIMESTAMPED into the same file as the sightings.
# Without that correlation "an isolate child existed" cannot be attributed to a cause:
# `boot_capture` loads the guest driver ~30 s after QEMU starts, so a sighting's t+ places
# it on one side or the other of the only guest action in the run.
echo "=== boot_capture phases (t+ seconds from witness start) ===" >> "$OUT"
"$REPO/scripts/bench/boot_capture.sh" "$TAG" 2>&1 \
  | while IFS= read -r line; do
      printf 't+%-4s %s\n' "$(( $(date +%s) - T0 ))" "$line" | tee -a "$OUT"
    done
BOOT_RC=${PIPESTATUS[0]}

sleep 2
kill -9 $SPID 2>/dev/null; trap - EXIT
SEEN=$(cat /tmp/e0_seen_$TAG 2>/dev/null || echo 0)

{
  echo "=== summary ==="
  echo "boot_capture rc: $BOOT_RC"
  echo "isolate-child samples: $SEEN   (0 = never seen alive)"
  echo "distinct isolate-child pids: $(grep -c '^=== ISOLATE CHILD pid ' "$OUT")"
} >> "$OUT"

say "boot_capture rc=$BOOT_RC ; isolate-child samples=$SEEN"
say "witness → $OUT"
N=$(grep -c '^=== ISOLATE CHILD pid ' "$OUT")
if [ "$N" -gt 0 ]; then
  say "★ $N DISTINCT ISOLATE CHILD(REN) EXISTED — see the t+ stamps for WHICH PHASE spawned each"
  grep -E '^=== ISOLATE CHILD pid |opening the device|guest is up|booting' "$OUT" | sed 's/^/    /'
else
  say "⊘ no isolate child ever existed"
fi
exit "$BOOT_RC"
