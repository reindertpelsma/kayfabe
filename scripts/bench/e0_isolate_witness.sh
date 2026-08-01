#!/usr/bin/env bash
# ★★★ E0/E0b's witness: does a GUEST action cause a REAL host RM verb?
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
  # ★★ THE ARCHIVE UNDER TEST IS NOT NECESSARILY HEAD. A bench once served a binary built
  # from a revision weeks behind the tree and every result was attributed to the wrong one.
  # This does not fail the run — a doc-only commit after the build is legitimate — it lists
  # exactly WHICH files differ, so a reader can judge instead of assuming.
  BINREV=$(strings "$BENCH/qemu-build/qemu-system-x86_64" 2>/dev/null \
           | grep -o 'kayfabe-rev:[0-9a-f]\{40\}' | sed 's/kayfabe-rev://' | sort -u | head -1)
  if [ -n "$BINREV" ] && [ "$BINREV" != "$REV" ]; then
    echo "⚠ ARCHIVE REV != SOURCE REV. Files differing ${BINREV:0:8}..${REV:0:8}:"
    git -C "$REPO" diff --name-only "$BINREV" "$REV" 2>/dev/null | sed 's/^/    /'
    echo "  ⊘ If any line above is under crates/ or Cargo.lock, this run does NOT measure $REV."
  else
    echo "archive rev == source rev"
  fi
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

# ★★ THE PLANE→RmMode MAPPING, ASSERTED ON HARDWARE. Nothing in the pure tests can see it:
# `isolate_factory` hands back a `Box<dyn IsolateFactory>` and the `RmMode` inside it is not
# observable from Rust. The child's own argv IS, and this is the only instrument that reads
# it. `IsolatePlane::Real` building a `RmMode::Loopback` factory would spawn a child that
# never touches an NVIDIA node and would otherwise look exactly like success here.
MAPPING=ok
case "$PLANE" in
  real|loopback)
    if [ "$N" -eq 0 ]; then
      MAPPING="★ FAILED: plane=$PLANE and NO isolate child ever existed"
    elif ! grep -q -- "--rm $PLANE" "$OUT"; then
      MAPPING="★ FAILED: a child exists but its argv does not carry '--rm $PLANE'"
    fi
    # And, for `real`, the RM nodes it can only hold if RmConnection::open succeeded.
    if [ "$PLANE" = real ] && [ "$MAPPING" = ok ]; then
      grep -q '/dev/nvidiactl' "$OUT" || MAPPING="★ FAILED: no /dev/nvidiactl descriptor"
      grep -q 'rw-s .*/dev/nvidia0' "$OUT" \
        || MAPPING="★ FAILED: no RM-served /dev/nvidia0 MAPPING (R6b never completed)"
    fi
    ;;
  stillborn|unset)
    [ "$N" -eq 0 ] || MAPPING="★ FAILED: plane=$PLANE spawned $N isolate child(ren)"
    ;;
esac
echo "plane->RmMode mapping check: $MAPPING" >> "$OUT"
say "plane->RmMode mapping check: $MAPPING"
[ "$MAPPING" = ok ] || BOOT_RC=4

if [ "$N" -gt 0 ]; then
  say "★ $N DISTINCT ISOLATE CHILD(REN) EXISTED — see the t+ stamps for WHICH PHASE spawned each"
  grep -E '^=== ISOLATE CHILD pid |opening the device|guest is up|booting' "$OUT" | sed 's/^/    /'
else
  say "⊘ no isolate child ever existed"
fi

# =====================================================================================
# ★★★ E0b — THE ATTRIBUTION, ASSERTED AND NOT LEFT TO A READER
# =====================================================================================
#
# E0's evidence contained both numbers and a human compared them. That is one step better
# than the FIRST version of this script, which latched only the first sighting and printed
# "★ AN ISOLATE CHILD EXISTED" — a sentence compatible with both the strong claim and the
# weak one — but it is still not a check. E0b's whole content is an ORDERING, so the
# ordering is what this asserts.
#
# ★★ **The timeline is not produced by the thing under test.** The sighting times come from
# scanning the host's /proc at 2 Hz; the phase times come from `boot_capture.sh`'s own
# stdout, stamped by this wrapper. Neither is written by the device, the archive, or the
# core. The device's own `isolates: N materialized` line (E1, below) is a corroborating
# reading and is deliberately NOT what decides this.
#
# ⊘ **It cannot pass vacuously.** For a plane that is supposed to spawn, a missing phase
# line, a missing sighting, or an unparseable stamp is a FAILURE — not a skip. The one
# thing worse than a red check here is a green one that measured nothing.
E0B="not applicable (plane=$PLANE spawns nothing by design)"
case "$PLANE" in
  real|loopback)
    # The MINIMUM over every sighting, not the first line in the file: the sampler appends
    # sightings directly while the phase lines arrive through a pipe, so file ORDER is racy
    # and only the stamps are authoritative.
    FIRST_T=$(sed -n 's/^=== ISOLATE CHILD pid .*(t+\([0-9]\{1,\}\)s from witness start).*/\1/p' "$OUT" \
              | sort -n | head -1)
    OPEN_T=$(sed -n 's/^t+\([0-9]\{1,\}\) *.*opening the device.*/\1/p' "$OUT" | head -1)
    if [ -z "$OPEN_T" ]; then
      E0B="★ FAILED: boot_capture never printed its 'opening the device' phase line, so
            there is no timeline to attribute a spawn against. ⊘ This is an INSTRUMENT
            failure, not a green."
    elif [ -z "$FIRST_T" ]; then
      E0B="★ FAILED: no isolate child was ever sighted, so the lazy spawn cannot be
            distinguished from a spawn that never happens (t_open=${OPEN_T}s)"
    elif [ "$FIRST_T" -lt "$OPEN_T" ]; then
      E0B="★ FAILED: the isolate child appeared at t+${FIRST_T}s, BEFORE the guest opened
            the device at t+${OPEN_T}s. The spawn is realize-time — this is exactly the
            state E0 measured (child t+3s, device open t+30-34s) and E0b exists to change."
    else
      E0B="ok — first isolate child at t+${FIRST_T}s, guest opened the device at
            t+${OPEN_T}s ⇒ the spawn FOLLOWS the guest's action"
    fi
    ;;
esac
echo "E0b lazy-spawn check: $E0B" >> "$OUT"
say "E0b lazy-spawn check: $E0B"
case "$E0B" in "★ FAILED"*) BOOT_RC=5 ;; esac

# =====================================================================================
# ★★★ E1 — THE DEVICE'S OWN REPORT OF ITS ISOLATE PLANE
# =====================================================================================
#
# `bench_rebuild_notes.md` §5 row 7: a FAILED real isolate was indistinguishable from a
# deliberately plane-less one at the seam, so a spawn failure presented to every layer above
# as "nothing happened". The archive now prints the census at teardown. This lifts those
# lines out of the QEMU log into the witness, so an evidence file carries them.
#
# ⊘ It is a REPORT, not the E0b check: the device writes it, so it cannot attribute
# anything. What it CAN do is turn a silent absence into a named line, which is E1's
# entire content.
QLOG=$BENCH/run_${TAG}_qemu.log
{
  echo "=== E1: the device's own isolate-plane census (from $QLOG) ==="
  if grep -q 'nvkvm: isolates:' "$QLOG" 2>/dev/null; then
    grep -E 'nvkvm: +isolates:|nvkvm: +isolate refusal' "$QLOG" | sed 's/^/  /'
  else
    echo "  ⊘ NO 'nvkvm: isolates:' LINE. Either this archive predates E1 (check the"
    echo "    embedded rev above) or the teardown report did not run — do not read the"
    echo "    absence as 'the plane is healthy'."
  fi
} >> "$OUT"
grep -E 'nvkvm: +isolates:|nvkvm: +isolate refusal' "$QLOG" 2>/dev/null | sed 's/^/    /'

exit "$BOOT_RC"
